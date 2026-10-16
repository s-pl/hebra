//! Hebra Cloud plans, quotas and metered usage. Community servers have no
//! plans: every limit here is skipped when `Server::plans` is `None`.
//!
//! - [`Plans`]: what each plan includes (`deploy/billing/plans.toml`);
//! - quota checks: called where a limit applies, with errors that say which
//!   limit was reached and how to raise it;
//! - usage: agent credits and storage, recorded per owner and month;
//! - [`stripe`]: checkout, customer portal, webhooks, usage reporting.

pub mod stripe;

use serde::{Deserialize, Serialize};

use crate::db::{Account, AccountKind};
use crate::error::{ApiError, ApiResult};
use crate::server::Server;

pub const METRIC_AGENT_CREDITS: &str = "agent_credits";
pub const METRIC_STORAGE_GB: &str = "storage_gb";
/// Credits above the allowance (billed through the Stripe meter).
pub const METRIC_AGENT_CREDITS_OVERAGE: &str = "agent_credits_overage";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub price_cents: i64,
    #[serde(default)]
    pub per_person: bool,
    #[serde(default)]
    pub contact: bool,
    /// People in an organization (`None`: unlimited).
    pub max_people: Option<i64>,
    pub max_private_repos: Option<i64>,
    #[serde(default)]
    pub storage_gb: i64,
    #[serde(default)]
    pub storage_gb_per_person: i64,
    #[serde(default)]
    pub agent_credits: i64,
    #[serde(default)]
    pub agent_credits_per_person: i64,
    #[serde(default)]
    pub merge_queue: bool,
    #[serde(default)]
    pub sso: bool,
    #[serde(default)]
    pub pitr_days: i64,
    pub uptime: Option<String>,
    pub support: Option<String>,
    /// Stripe price id for the subscription.
    #[serde(default, skip_serializing)]
    pub stripe_price: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plans {
    pub currency: String,
    pub credit_price_cents: i64,
    pub model_markup_percent: i64,
    pub credits_per_run_limit: i64,
    pub storage_overage_cents_per_gb: i64,
    #[serde(rename = "plan")]
    pub plans: Vec<Plan>,
}

impl Plans {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text)
    }

    /// The plans shipped with Hebra (same as `deploy/billing/plans.toml`).
    #[must_use]
    pub fn builtin() -> Self {
        Self::parse(include_str!("../../../../deploy/billing/plans.toml"))
            .expect("built-in plans are valid")
    }

    fn parse(text: &str) -> anyhow::Result<Self> {
        let plans: Self = toml::from_str(text)?;
        anyhow::ensure!(plans.plan("free").is_some(), "plans need a `free` plan");
        Ok(plans)
    }

    #[must_use]
    pub fn plan(&self, id: &str) -> Option<&Plan> {
        self.plans.iter().find(|p| p.id == id)
    }
}

/// What an owner's plan allows now.
#[derive(Debug, Clone, Serialize)]
pub struct Allowance {
    pub plan: Plan,
    pub people: i64,
    pub agent_credits: i64,
    pub storage_gb: i64,
    pub spending_limit_cents: i64,
    pub status: String,
}

impl Server {
    /// The plan of an owner (user or organization); `free` when there is no
    /// subscription or it lapsed.
    pub(crate) async fn allowance(&self, owner: &Account) -> ApiResult<Option<Allowance>> {
        let Some(plans) = &self.plans else {
            return Ok(None);
        };
        let sub = self.db.subscription(&owner.id).await?;
        let (plan_id, status, limit) = match &sub {
            Some(s) if matches!(s.status.as_str(), "active" | "trialing") => {
                (s.plan.as_str(), s.status.clone(), s.spending_limit_cents)
            }
            Some(s) => ("free", s.status.clone(), 0),
            None => ("free", "free".to_owned(), 0),
        };
        let plan = plans
            .plan(plan_id)
            .or_else(|| plans.plan("free"))
            .cloned()
            .ok_or_else(|| ApiError::internal("billing", "no free plan"))?;
        let people = match owner.kind {
            AccountKind::Org => self.db.count_members(&owner.id).await?.max(1),
            _ => 1,
        };
        Ok(Some(Allowance {
            agent_credits: plan.agent_credits + plan.agent_credits_per_person * people,
            storage_gb: plan.storage_gb + plan.storage_gb_per_person * people,
            people,
            spending_limit_cents: limit,
            status,
            plan,
        }))
    }

    fn upgrade_hint(&self) -> String {
        format!("see {}/pricing", self.public_url)
    }

    /// Before creating a private repository.
    pub(crate) async fn check_private_repo_quota(&self, owner: &Account) -> ApiResult<()> {
        let Some(a) = self.allowance(owner).await? else {
            return Ok(());
        };
        if let Some(max) = a.plan.max_private_repos
            && self.db.count_private_repos(&owner.id).await? >= max
        {
            return Err(ApiError::payment(format!(
                "the {} plan includes {max} private repositories; {}",
                a.plan.name,
                self.upgrade_hint()
            )));
        }
        Ok(())
    }

    /// Before adding a person to an organization.
    pub(crate) async fn check_people_quota(&self, org: &Account) -> ApiResult<()> {
        let Some(a) = self.allowance(org).await? else {
            return Ok(());
        };
        if let Some(max) = a.plan.max_people
            && a.people >= max
        {
            return Err(ApiError::payment(format!(
                "the {} plan includes {max} people; {}",
                a.plan.name,
                self.upgrade_hint()
            )));
        }
        Ok(())
    }

    /// Credits an owner can still spend this period (allowance plus what the
    /// spending limit pays for).
    pub(crate) async fn credits_left(&self, owner: &Account) -> ApiResult<Option<i64>> {
        let Some(plans) = &self.plans else {
            return Ok(None);
        };
        let Some(a) = self.allowance(owner).await? else {
            return Ok(None);
        };
        let used = self.db.usage_of(&owner.id, METRIC_AGENT_CREDITS).await?;
        let paid = a.spending_limit_cents / plans.credit_price_cents.max(1);
        Ok(Some((a.agent_credits + paid - used).max(0)))
    }

    /// Records credits used by an agent run. The part above the plan's
    /// allowance is also recorded as overage, which is billed.
    pub(crate) async fn charge_credits(
        &self,
        owner: &Account,
        credits: i64,
        reference: &str,
    ) -> ApiResult<()> {
        let Some(a) = self.allowance(owner).await? else {
            return Ok(());
        };
        if credits <= 0 {
            return Ok(());
        }
        let before = self.db.usage_of(&owner.id, METRIC_AGENT_CREDITS).await?;
        self.db
            .record_usage(&owner.id, METRIC_AGENT_CREDITS, credits, reference)
            .await?;
        let over = |used: i64| (used - a.agent_credits).max(0);
        let overage = over(before + credits) - over(before);
        if overage > 0 {
            self.db
                .record_usage(&owner.id, METRIC_AGENT_CREDITS_OVERAGE, overage, reference)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Plans;

    /// The web's pricing page falls back to a copy of the plans; it must say
    /// the same prices and allowances as the file the server bills with.
    #[test]
    fn web_copy_matches_plans_toml() {
        let web = include_str!("../../../../apps/web/lib/plans.ts");
        let block = |id: &str| {
            let start = web
                .find(&format!("id: \"{id}\""))
                .unwrap_or_else(|| panic!("plan `{id}` missing from apps/web/lib/plans.ts"));
            let end = web[start..].find("},").map_or(web.len(), |e| start + e);
            web[start..end].to_owned()
        };
        for plan in Plans::builtin().plans {
            let text = block(&plan.id);
            for (field, value) in [
                ("price_cents", plan.price_cents),
                ("storage_gb", plan.storage_gb),
                ("storage_gb_per_person", plan.storage_gb_per_person),
                ("agent_credits", plan.agent_credits),
                ("agent_credits_per_person", plan.agent_credits_per_person),
                ("pitr_days", plan.pitr_days),
            ] {
                assert!(
                    text.contains(&format!("{field}: {value},")),
                    "plan `{}`: {field} should be {value} in apps/web/lib/plans.ts",
                    plan.id
                );
            }
            for (field, value) in [
                ("per_person", plan.per_person),
                ("merge_queue", plan.merge_queue),
                ("sso", plan.sso),
            ] {
                assert!(
                    text.contains(&format!("{field}: {value},")),
                    "plan `{}`: {field} should be {value} in apps/web/lib/plans.ts",
                    plan.id
                );
            }
        }
    }
}
