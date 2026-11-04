//! The fleet orchestrator: Heddle, running inside the server.
//!
//! One replica at a time (a lease), every few seconds, for each owner with
//! a fleet it:
//!
//! 1. reconciles with the provider: machines the provider no longer has are
//!    gone, machines with the fleet's tag that nobody remembers are deleted
//!    (they would cost without working), creates that never finished fail;
//! 2. follows each machine's runner: a machine is ready once its runner
//!    connected, and one that does not register in time is deleted;
//! 3. asks Heddle's planner what to do with the queue, the machines, the
//!    provider's live prices and the owner's own history (typical durations,
//!    arrivals in the last hour), and does it: new machines get a one-time
//!    runner token in their first-boot script, idle machines are deleted
//!    along with their runner;
//! 4. keeps what each machine has cost so far, as the provider bills.
//!
//! Everything it decides is written to the fleet's event log, with why.

pub mod providers;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use hebra_heddle::plan::Heddle;
use hebra_heddle::{Action, Billing, Goal, Job, Machine, MachineType, Params, Policy};
use serde_json::{Value, json};

use crate::db::{self, FleetRow, MachineRow, NewMachine, NewRunnerToken};
use crate::error::{ApiError, ApiResult};
use crate::identity::secret;
use crate::server::Server;
use providers::{Bootstrap, Provider};

/// How long a machine may take from creation to its runner connecting.
const DEFAULT_BOOT_TIMEOUT_SECS: i64 = 15 * 60;
/// Provider catalogs (types and prices) are fetched this often.
const CATALOG_TTL: Duration = Duration::from_secs(600);
/// A machine the provider does not list is gone after this long.
const GONE_AFTER_MS: i64 = 90_000;
/// Seconds a check is assumed to take before it has a history.
const DEFAULT_SECS: f64 = 120.0;

/// What the orchestrator remembers between rounds (on the replica that
/// holds the lease; it is rebuilt from the database if that changes).
#[derive(Default)]
pub struct FleetMemory {
    /// Machine id -> since when it has been idle (ms).
    idle_since: HashMap<String, i64>,
    /// Owner -> (when fetched, catalog).
    catalogs: HashMap<String, (Instant, Vec<MachineType>)>,
    /// Owner -> last error reported, to log it once.
    last_error: HashMap<String, String>,
}

/// The place a fleet's API token is sealed under.
pub(crate) fn credentials_place(owner_id: &str) -> String {
    format!("fleet/{owner_id}/token")
}

/// Heddle's parameters for a fleet's settings.
pub(crate) fn params(config: &Value) -> Params {
    let goal = config["goal"]
        .as_str()
        .and_then(Goal::parse)
        .unwrap_or(Goal::Balanced);
    let mut p = goal.params();
    if let Some(n) = config["max_machines"].as_u64() {
        p.max_machines = usize::try_from(n).unwrap_or(4);
    } else {
        p.max_machines = 4;
    }
    if let Some(x) = config["max_spend_per_hour"].as_f64() {
        p.max_spend_per_hour = x;
    }
    if let Some(x) = config["time_value_per_hour"].as_f64() {
        p.time_value_per_hour = x;
    }
    if let Some(n) = config["min_idle"].as_u64() {
        p.min_idle = usize::try_from(n).unwrap_or(0);
    }
    p
}

/// What keeping a machine `secs` costs, as it is billed (micros).
#[must_use]
pub fn billed_micros(price_micros: i64, billing: &str, secs: i64) -> i64 {
    let secs = secs.max(0);
    let billed = if billing == "second" {
        secs.max(60)
    } else {
        ((secs + 3599) / 3600).max(1) * 3600
    };
    i128::from(price_micros)
        .saturating_mul(i128::from(billed))
        .checked_div(3600)
        .and_then(|v| i64::try_from(v).ok())
        .unwrap_or(i64::MAX)
}

fn billing_name(b: Billing) -> &'static str {
    match b {
        Billing::Second => "second",
        Billing::Hour => "hour",
    }
}

/// The tag machines of an owner's fleet carry (`[a-z0-9-]`).
fn tag(handle: &str) -> String {
    handle
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

impl Server {
    /// The provider of a fleet, with its token opened.
    pub(crate) fn fleet_provider(&self, fleet: &FleetRow) -> ApiResult<Provider> {
        let token = match &fleet.credentials {
            Some(sealed) => Some(
                String::from_utf8_lossy(
                    &self
                        .sealer
                        .open(&credentials_place(&fleet.owner_id), sealed)?,
                )
                .into_owned(),
            ),
            None => None,
        };
        Provider::new(&fleet.provider, &fleet.config, token, self.http.clone())
    }

    /// The machine types a fleet may use, with the provider's prices.
    pub(crate) async fn fleet_catalog(
        &self,
        fleet: &FleetRow,
        provider: &Provider,
    ) -> ApiResult<Vec<MachineType>> {
        let cached = self
            .fleet_memory
            .lock()
            .ok()
            .and_then(|m| m.catalogs.get(&fleet.owner_id).cloned())
            .filter(|(at, _)| at.elapsed() < CATALOG_TTL);
        let mut types = match cached {
            Some((_, types)) => types,
            None => {
                let fresh = provider.catalog().await?;
                if let Ok(mut m) = self.fleet_memory.lock() {
                    m.catalogs
                        .insert(fleet.owner_id.clone(), (Instant::now(), fresh.clone()));
                }
                fresh
            }
        };
        let allowed: Vec<String> = fleet.config["types"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect();
        if !allowed.is_empty() && fleet.provider != "docker" {
            types.retain(|t| allowed.contains(&t.name));
        }
        Ok(types)
    }

    pub(crate) fn forget_fleet_catalog(&self, owner_id: &str) {
        if let Ok(mut m) = self.fleet_memory.lock() {
            m.catalogs.remove(owner_id);
        }
    }

    /// One pass over every fleet.
    pub(crate) async fn run_fleets(&self) -> ApiResult<()> {
        for owner in self.db.active_fleets().await? {
            let result =
                tokio::time::timeout(Duration::from_secs(60), self.run_fleet(&owner)).await;
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e.message().to_owned()),
                Err(_) => {
                    Some("a round took more than a minute (the provider is slow?)".to_owned())
                }
            };
            let repeated = self
                .fleet_memory
                .lock()
                .ok()
                .is_some_and(|m| m.last_error.get(&owner) == error.as_ref());
            if let Some(message) = &error
                && !repeated
            {
                tracing::warn!(owner, error = message, "fleet");
                self.db.fleet_event(&owner, "error", None, message).await?;
            }
            if let Ok(mut m) = self.fleet_memory.lock() {
                match error {
                    Some(e) => m.last_error.insert(owner, e),
                    None => m.last_error.remove(&owner),
                };
            }
        }
        Ok(())
    }

    /// Deletes a machine at its provider and forgets its runner.
    pub(crate) async fn stop_machine(
        &self,
        provider: &Provider,
        m: &MachineRow,
        reason: &str,
    ) -> ApiResult<()> {
        self.db
            .set_machine_state(&m.id, "deleting", Some(reason))
            .await?;
        if let Some(pid) = &m.provider_id {
            provider.delete(pid).await?;
        }
        self.retire_machine(m, "gone", reason).await?;
        self.db
            .fleet_event(&m.owner_id, "stop", Some(&m.name), reason)
            .await
    }

    async fn retire_machine(&self, m: &MachineRow, state: &str, reason: &str) -> ApiResult<()> {
        let secs = (db::now_ms() - m.created_ms) / 1000;
        self.db
            .set_machine_cost(&m.id, billed_micros(m.price_micros, &m.billing, secs))
            .await?;
        self.db
            .set_machine_state(&m.id, state, Some(reason))
            .await?;
        if let Some(r) = self
            .db
            .runners(&m.owner_id)
            .await?
            .into_iter()
            .find(|r| r.name == m.name)
        {
            self.db.disable_account(&r.account_id).await?;
            self.db.delete_runner(&r.id).await?;
            self.caches.principals.clear();
        }
        if let Ok(mut mem) = self.fleet_memory.lock() {
            mem.idle_since.remove(&m.id);
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn run_fleet(&self, owner_id: &str) -> ApiResult<()> {
        let Some(fleet) = self.db.fleet(owner_id).await? else {
            return Ok(());
        };
        let owner = self
            .db
            .account(owner_id)
            .await?
            .ok_or_else(|| ApiError::not_found("fleet owner"))?;
        let provider = self.fleet_provider(&fleet)?;
        let tag = tag(&owner.handle);
        let now = db::now_ms();
        let config = &fleet.config;

        // 1. What the provider has.
        let listed = provider.list(&tag).await?;
        let mut live = self.db.machines(owner_id, false).await?;
        let known: HashSet<&str> = live
            .iter()
            .filter_map(|m| m.provider_id.as_deref())
            .collect();
        for l in listed
            .iter()
            .filter(|l| !known.contains(l.provider_id.as_str()))
        {
            // Created by a round that crashed before recording it, or left
            // over: it costs and nobody uses it.
            provider.delete(&l.provider_id).await?;
            self.db
                .fleet_event(
                    owner_id,
                    "orphan",
                    Some(&l.name),
                    "deleted a machine with this fleet's tag that the server did not know",
                )
                .await?;
        }
        let present: HashMap<&str, bool> = listed
            .iter()
            .map(|l| (l.provider_id.as_str(), l.running))
            .collect();
        let mut gone = Vec::new();
        for m in &live {
            match &m.provider_id {
                None if now - m.created_ms > 5 * 60_000 => {
                    self.retire_machine(m, "failed", "the provider never confirmed it")
                        .await?;
                    gone.push(m.id.clone());
                }
                Some(pid)
                    if !present.contains_key(pid.as_str())
                        && now - m.created_ms > GONE_AFTER_MS =>
                {
                    self.retire_machine(m, "gone", "the provider no longer has it")
                        .await?;
                    self.db
                        .fleet_event(
                            owner_id,
                            "gone",
                            Some(&m.name),
                            "the provider no longer has it",
                        )
                        .await?;
                    gone.push(m.id.clone());
                }
                _ => {}
            }
        }
        live.retain(|m| !gone.contains(&m.id));

        // 2. Runners.
        let runners = self.db.runners(owner_id).await?;
        let busy: HashSet<String> = self
            .db
            .busy_runners(owner_id)
            .await?
            .into_iter()
            .map(|(runner, _)| runner)
            .collect();
        let timeout = config["boot_timeout_secs"]
            .as_i64()
            .unwrap_or(DEFAULT_BOOT_TIMEOUT_SECS);
        let catalog = self.fleet_catalog(&fleet, &provider).await?;
        let mut view = Vec::new();
        let mut busy_count = HashMap::new();
        for m in &live {
            let runner = runners
                .iter()
                .find(|r| r.machine.as_deref() == Some(m.name.as_str()) || r.name == m.name);
            let online = runner.is_some_and(|r| r.online);
            let working = runner.is_some_and(|r| busy.contains(&r.id));
            if m.state == "booting" && online {
                self.db.set_machine_state(&m.id, "ready", None).await?;
                self.db
                    .fleet_event(
                        owner_id,
                        "ready",
                        Some(&m.name),
                        &format!(
                            "its runner connected after {}s",
                            (now - m.created_ms) / 1000
                        ),
                    )
                    .await?;
            } else if m.state == "booting" && now - m.created_ms > timeout * 1000 {
                self.stop_machine(
                    &provider,
                    m,
                    &format!("its runner did not connect within {}s", timeout),
                )
                .await?;
                continue;
            }
            let idle_since = if working || !online {
                if let Ok(mut mem) = self.fleet_memory.lock() {
                    mem.idle_since.remove(&m.id);
                }
                None
            } else {
                self.fleet_memory
                    .lock()
                    .ok()
                    .map(|mut mem| *mem.idle_since.entry(m.id.clone()).or_insert(now))
            };
            if working {
                *busy_count.entry(m.kind.clone()).or_insert(0) += 1;
            }
            let boot = catalog
                .iter()
                .find(|t| t.name == m.kind)
                .map_or(60.0, |t| t.boot_secs);
            #[allow(clippy::cast_precision_loss)]
            let secs = |ms: i64| ms as f64 / 1000.0;
            view.push(Machine {
                id: m.id.clone(),
                kind: m.kind.clone(),
                created: secs(m.created_ms),
                ready_at: if online {
                    secs(m.ready_ms.unwrap_or(m.created_ms))
                } else {
                    (secs(m.created_ms) + boot).max(secs(now) + 1.0)
                },
                busy_until: working.then_some(secs(now) + DEFAULT_SECS / 2.0),
                idle_since: idle_since.map(secs),
            });
            // What it has cost so far.
            self.db
                .set_machine_cost(
                    &m.id,
                    billed_micros(m.price_micros, &m.billing, (now - m.created_ms) / 1000),
                )
                .await?;
        }

        // 3. Plan.
        let durations = self.db.typical_durations(owner_id).await?;
        let queued = self.db.queued_runs(owner_id).await?;
        #[allow(clippy::cast_precision_loss)]
        let jobs: Vec<Job> = queued
            .iter()
            .filter(|q| !q.blocked)
            .map(|q| Job {
                id: format!("{} {}", q.repo, q.name),
                labels: q.labels.clone(),
                reference_secs: durations
                    .get(q.name.split(" (").next().unwrap_or(&q.name))
                    .copied()
                    .unwrap_or(DEFAULT_SECS),
                queued_at: (now - q.waiting_ms) as f64 / 1000.0,
            })
            .collect();
        let mut params = params(config);
        #[allow(clippy::cast_precision_loss)]
        {
            params.recent_arrivals = self
                .db
                .recent_arrivals(owner_id, now - 3_600_000)
                .await?
                .into_iter()
                .map(|t| t as f64 / 1000.0)
                .collect();
        }
        #[allow(clippy::cast_precision_loss)]
        let now_secs = now as f64 / 1000.0;
        let actions: Vec<Action> = if fleet.enabled {
            Heddle.plan(now_secs, &catalog, &view, &jobs, &params)
        } else {
            // Switched off: idle machines go, busy ones finish first.
            view.iter()
                .filter(|m| m.busy_until.is_none())
                .map(|m| Action::Destroy {
                    id: m.id.clone(),
                    reason: "the fleet is switched off".to_owned(),
                })
                .collect()
        };

        for action in actions {
            match action {
                Action::Create { kind, reason, .. } => {
                    let Some(t) = catalog.iter().find(|t| t.name == kind) else {
                        continue;
                    };
                    self.start_machine(&fleet, &owner, &provider, &tag, t, &reason)
                        .await?;
                }
                Action::Destroy { id, reason } => {
                    if let Some(m) = live.iter().find(|m| m.id == id) {
                        self.stop_machine(&provider, m, &reason).await?;
                    }
                }
            }
        }
        let _ = busy_count;
        Ok(())
    }

    async fn start_machine(
        &self,
        fleet: &FleetRow,
        owner: &db::Account,
        provider: &Provider,
        tag: &str,
        kind: &MachineType,
        reason: &str,
    ) -> ApiResult<()> {
        let mut suffix = [0u8; 4];
        rand::fill(&mut suffix);
        let name = format!("heddle-{}", hex::encode(suffix));
        let labels = providers::fleet_labels(&fleet.config);
        let token = secret::token(crate::api::RUNNER_TOKEN_PREFIX);
        self.db
            .create_runner_token(&NewRunnerToken {
                hash: &secret::hash(&token),
                owner_id: &owner.id,
                labels: &labels,
                ephemeral: false,
                uses: 1,
                expires_at: db::now_ms() + 2 * 3_600_000,
                by: "heddle",
            })
            .await?;
        #[allow(clippy::cast_possible_truncation)]
        let price_micros = (kind.price_per_hour * 1_000_000.0).round() as i64;
        let id = self
            .db
            .create_machine(&NewMachine {
                owner_id: &owner.id,
                provider: &fleet.provider,
                name: &name,
                kind: &kind.name,
                price_micros,
                billing: billing_name(kind.billing),
                reason,
            })
            .await?;
        let server = fleet.config["server_url"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.public_url)
            .to_owned();
        let boot = Bootstrap {
            server: &server,
            token: &token,
            name: &name,
            labels: &labels,
            version: fleet.config["version"].as_str().filter(|v| !v.is_empty()),
            docker: fleet.config["docker"].as_bool().unwrap_or(true),
        };
        match provider.create(kind, &name, tag, &boot).await {
            Ok(created) => {
                self.db
                    .set_machine_provider(&id, &created.provider_id, created.address.as_deref())
                    .await?;
                self.db
                    .fleet_event(
                        &owner.id,
                        "start",
                        Some(&name),
                        &format!(
                            "{} ({} CPU, {}/h, billed by the {}): {reason}",
                            kind.name,
                            kind.cpus,
                            kind.price_per_hour,
                            billing_name(kind.billing)
                        ),
                    )
                    .await
            }
            Err(e) => {
                self.db
                    .set_machine_state(&id, "failed", Some(e.message()))
                    .await?;
                Err(e)
            }
        }
    }

    /// Heddle's policies on an owner's own recent history, with the fleet's
    /// prices: what each would have cost and how long checks would have waited.
    pub(crate) async fn simulate_fleet(&self, owner_id: &str, days: i64) -> ApiResult<Value> {
        use hebra_heddle::sim::{Trace, TraceJob, simulate};
        let since = db::now_ms() - days.clamp(1, 30) * 86_400_000;
        let history = self.db.owner_history(owner_id, since).await?;
        #[allow(clippy::cast_precision_loss)]
        let jobs: Vec<TraceJob> = history
            .iter()
            .filter_map(|(queued, started, finished, labels)| {
                Some(TraceJob {
                    at: *queued as f64 / 1000.0,
                    secs: ((*finished)? - (*started)?) as f64 / 1000.0,
                    labels: labels.clone(),
                })
            })
            .collect();
        let mut trace = Trace { jobs };
        trace.jobs.sort_by(|a, b| a.at.total_cmp(&b.at));
        let fleet = self.db.fleet(owner_id).await?;
        let catalog = match &fleet {
            Some(f) => match self.fleet_provider(f) {
                Ok(p) => self
                    .fleet_catalog(f, &p)
                    .await
                    .unwrap_or_else(|_| hebra_heddle::Catalog::example().types),
                Err(_) => hebra_heddle::Catalog::example().types,
            },
            None => hebra_heddle::Catalog::example().types,
        };
        // Jobs whose labels no type has would never run: give them the first.
        let all: HashSet<String> = catalog.iter().flat_map(|t| t.labels.clone()).collect();
        for j in &mut trace.jobs {
            j.labels.retain(|l| all.contains(l));
        }
        if trace.jobs.is_empty() {
            return Ok(json!({ "jobs": 0, "reports": [], "catalog": catalog }));
        }
        let mut params = fleet
            .as_ref()
            .map_or_else(Params::default, |f| params(&f.config));
        params.arrivals_per_hour = trace.rate_per_hour();
        let reports: Vec<_> = hebra_heddle::plan::baselines(&catalog)
            .iter()
            .map(|p| simulate(&trace, &catalog, p.as_ref(), &params))
            .collect();
        Ok(
            json!({ "jobs": trace.jobs.len(), "days": days, "reports": reports, "catalog": catalog }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_rounds_like_providers() {
        assert_eq!(billed_micros(10_000, "hour", 10), 10_000);
        assert_eq!(billed_micros(10_000, "hour", 3601), 20_000);
        assert_eq!(billed_micros(36_000, "second", 30), 600);
        assert_eq!(billed_micros(36_000, "second", 120), 1200);
        assert_eq!(tag("Acme_Corp"), "acme-corp");
        let p = params(&json!({ "goal": "cheap", "max_machines": 9 }));
        assert_eq!(p.max_machines, 9);
        assert!(p.time_value_per_hour < 1.0);
    }
}
