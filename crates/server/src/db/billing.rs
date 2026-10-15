//! Subscriptions and metered usage (Cloud edition).

use serde::Serialize;

use super::{Db, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

#[derive(Debug, Clone, Serialize)]
pub struct Subscription {
    pub owner_id: String,
    pub plan: String,
    /// `active`, `trialing`, `past_due`, `canceled`, `incomplete`.
    pub status: String,
    #[serde(skip)]
    pub stripe_customer: Option<String>,
    #[serde(skip)]
    pub stripe_subscription: Option<String>,
    pub seats: i64,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub spending_limit_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageLine {
    pub metric: String,
    pub quantity: i64,
}

/// The billing period key for a timestamp: `YYYY-MM` (calendar months;
/// subscriptions renewing mid-month are prorated by Stripe).
#[must_use]
pub fn period_of(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.strftime("%Y-%m").to_string())
        .unwrap_or_default()
}

impl Db {
    pub async fn subscription(&self, owner_id: &str) -> ApiResult<Option<Subscription>> {
        type Row = (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
            Option<i64>,
            Option<i64>,
            i64,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT owner_id, plan, status, stripe_customer, stripe_subscription, seats, period_start, period_end,
                    spending_limit_cents
             FROM subscriptions WHERE owner_id = $1",
        )
        .bind(owner_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(
            |(
                owner_id,
                plan,
                status,
                stripe_customer,
                stripe_subscription,
                seats,
                start,
                end,
                limit,
            )| Subscription {
                owner_id,
                plan,
                status,
                stripe_customer,
                stripe_subscription,
                seats,
                period_start: start.map(rfc3339),
                period_end: end.map(rfc3339),
                spending_limit_cents: limit,
            },
        ))
    }

    pub async fn subscription_by_customer(&self, customer: &str) -> ApiResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT owner_id FROM subscriptions WHERE stripe_customer = $1")
                .bind(customer)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    /// Creates or updates the subscription of an owner.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_subscription(
        &self,
        owner_id: &str,
        plan: &str,
        status: &str,
        stripe_customer: Option<&str>,
        stripe_subscription: Option<&str>,
        seats: i64,
        period_start: Option<i64>,
        period_end: Option<i64>,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO subscriptions (owner_id, plan, status, stripe_customer, stripe_subscription, seats,
                                        period_start, period_end, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (owner_id) DO UPDATE SET plan = excluded.plan, status = excluded.status,
               stripe_customer = COALESCE(excluded.stripe_customer, subscriptions.stripe_customer),
               stripe_subscription = COALESCE(excluded.stripe_subscription, subscriptions.stripe_subscription),
               seats = excluded.seats, period_start = excluded.period_start, period_end = excluded.period_end,
               updated_at = excluded.updated_at",
        )
        .bind(owner_id)
        .bind(plan)
        .bind(status)
        .bind(stripe_customer)
        .bind(stripe_subscription)
        .bind(seats)
        .bind(period_start)
        .bind(period_end)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_spending_limit(&self, owner_id: &str, cents: i64) -> ApiResult<()> {
        sqlx::query("UPDATE subscriptions SET spending_limit_cents = $1, updated_at = $2 WHERE owner_id = $3")
            .bind(cents)
            .bind(now_ms())
            .bind(owner_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Adds usage to the owner's current period and keeps the event (for
    /// reporting to Stripe and for audit).
    pub async fn record_usage(
        &self,
        owner_id: &str,
        metric: &str,
        quantity: i64,
        reference: &str,
    ) -> ApiResult<()> {
        let now = now_ms();
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO usage (owner_id, period, metric, quantity) VALUES ($1, $2, $3, $4)
             ON CONFLICT (owner_id, period, metric) DO UPDATE SET quantity = usage.quantity + excluded.quantity",
        )
        .bind(owner_id)
        .bind(period_of(now))
        .bind(metric)
        .bind(quantity)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO usage_events (id, owner_id, metric, quantity, reference, at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(new_id("use"))
        .bind(owner_id)
        .bind(metric)
        .bind(quantity)
        .bind(reference)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Usage of the current period.
    pub async fn usage(&self, owner_id: &str) -> ApiResult<Vec<UsageLine>> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT metric, quantity FROM usage WHERE owner_id = $1 AND period = $2 ORDER BY metric")
                .bind(owner_id)
                .bind(period_of(now_ms()))
                .fetch_all(self.pool())
                .await?;
        Ok(rows
            .into_iter()
            .map(|(metric, quantity)| UsageLine { metric, quantity })
            .collect())
    }

    pub async fn usage_of(&self, owner_id: &str, metric: &str) -> ApiResult<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT CAST(COALESCE(SUM(quantity), 0) AS BIGINT) FROM usage WHERE owner_id = $1 AND period = $2 AND metric = $3",
        )
        .bind(owner_id)
        .bind(period_of(now_ms()))
        .bind(metric)
        .fetch_one(self.pool())
        .await?)
    }

    /// Usage events not yet reported to the payment provider.
    pub async fn unreported_usage(
        &self,
        metric: &str,
        limit: i64,
    ) -> ApiResult<Vec<(String, String, String, i64)>> {
        Ok(sqlx::query_as(
            "SELECT id, owner_id, metric, quantity FROM usage_events
             WHERE reported_at IS NULL AND metric = $1 ORDER BY at LIMIT $2",
        )
        .bind(metric)
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn mark_usage_reported(&self, id: &str) -> ApiResult<()> {
        sqlx::query("UPDATE usage_events SET reported_at = $1 WHERE id = $2")
            .bind(now_ms())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Records a payment-provider event id; false if it was already seen
    /// (webhooks are delivered at least once).
    pub async fn first_time_event(&self, id: &str) -> ApiResult<bool> {
        Ok(sqlx::query(
            "INSERT INTO stripe_events (id, received_at) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(now_ms())
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }
}
