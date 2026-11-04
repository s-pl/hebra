//! Fleets, their machines, and what the orchestrator decided.

use serde::Serialize;
use sqlx::Row;

use super::{Db, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

#[derive(Debug, Clone)]
pub struct FleetRow {
    pub owner_id: String,
    pub enabled: bool,
    pub provider: String,
    pub config: serde_json::Value,
    pub credentials: Option<String>,
    pub updated_at: i64,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MachineRow {
    pub id: String,
    #[serde(skip)]
    pub owner_id: String,
    pub provider: String,
    pub provider_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub state: String,
    pub price_micros: i64,
    pub billing: String,
    pub cost_micros: i64,
    pub address: Option<String>,
    pub reason: Option<String>,
    #[serde(skip)]
    pub created_ms: i64,
    #[serde(skip)]
    pub ready_ms: Option<i64>,
    #[serde(skip)]
    pub ended_ms: Option<i64>,
    pub created_at: String,
    pub ready_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetEvent {
    pub at: String,
    pub kind: String,
    pub machine: Option<String>,
    pub detail: String,
}

/// A check on one of the fleet's machines.
#[derive(Debug, Clone, Serialize)]
pub struct FleetRun {
    pub machine: String,
    pub name: String,
    pub repo: String,
    pub state: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

pub struct NewMachine<'a> {
    pub owner_id: &'a str,
    pub provider: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub price_micros: i64,
    pub billing: &'a str,
    pub reason: &'a str,
}

const MACHINE_COLUMNS: &str = "id, owner_id, provider, provider_id, name, kind, state, price_micros, billing, cost_micros, address, reason, created_at, ready_at, ended_at";

fn machine_row(r: &sqlx::any::AnyRow) -> Result<MachineRow, sqlx::Error> {
    let created: i64 = r.try_get("created_at")?;
    let ready: Option<i64> = r.try_get("ready_at")?;
    let ended: Option<i64> = r.try_get("ended_at")?;
    Ok(MachineRow {
        id: r.try_get("id")?,
        owner_id: r.try_get("owner_id")?,
        provider: r.try_get("provider")?,
        provider_id: r.try_get("provider_id")?,
        name: r.try_get("name")?,
        kind: r.try_get("kind")?,
        state: r.try_get("state")?,
        price_micros: r.try_get("price_micros")?,
        billing: r.try_get("billing")?,
        cost_micros: r.try_get("cost_micros")?,
        address: r.try_get("address")?,
        reason: r.try_get("reason")?,
        created_ms: created,
        ready_ms: ready,
        ended_ms: ended,
        created_at: rfc3339(created),
        ready_at: ready.map(rfc3339),
        ended_at: ended.map(rfc3339),
    })
}

impl Db {
    pub async fn fleet(&self, owner_id: &str) -> ApiResult<Option<FleetRow>> {
        let row = sqlx::query(
            "SELECT owner_id, enabled, provider, config, credentials, updated_at, updated_by FROM fleets WHERE owner_id = $1",
        )
        .bind(owner_id)
        .fetch_optional(self.pool())
        .await?;
        let Some(r) = row else { return Ok(None) };
        let config: String = r.try_get("config")?;
        Ok(Some(FleetRow {
            owner_id: r.try_get("owner_id")?,
            enabled: r.try_get::<i64, _>("enabled")? != 0,
            provider: r.try_get("provider")?,
            config: serde_json::from_str(&config).unwrap_or_default(),
            credentials: r.try_get("credentials")?,
            updated_at: r.try_get("updated_at")?,
            updated_by: r.try_get("updated_by")?,
        }))
    }

    /// Owners with an enabled fleet, or with machines still alive.
    pub async fn active_fleets(&self) -> ApiResult<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT owner_id FROM fleets WHERE enabled = 1
             UNION SELECT DISTINCT owner_id FROM machines WHERE state NOT IN ('gone', 'failed')",
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn put_fleet(
        &self,
        owner_id: &str,
        enabled: bool,
        provider: &str,
        config: &serde_json::Value,
        credentials: Option<&str>,
        by: &str,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO fleets (owner_id, enabled, provider, config, credentials, updated_at, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (owner_id) DO UPDATE SET enabled = excluded.enabled, provider = excluded.provider,
                config = excluded.config, credentials = COALESCE(excluded.credentials, fleets.credentials),
                updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        )
        .bind(owner_id)
        .bind(i64::from(enabled))
        .bind(provider)
        .bind(config.to_string())
        .bind(credentials)
        .bind(now_ms())
        .bind(by)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn create_machine(&self, m: &NewMachine<'_>) -> ApiResult<String> {
        let id = new_id("mch");
        sqlx::query(
            "INSERT INTO machines (id, owner_id, provider, name, kind, state, price_micros, billing, reason, created_at)
             VALUES ($1, $2, $3, $4, $5, 'provisioning', $6, $7, $8, $9)",
        )
        .bind(&id)
        .bind(m.owner_id)
        .bind(m.provider)
        .bind(m.name)
        .bind(m.kind)
        .bind(m.price_micros)
        .bind(m.billing)
        .bind(m.reason)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn machines(
        &self,
        owner_id: &str,
        include_ended: bool,
    ) -> ApiResult<Vec<MachineRow>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE owner_id = $1 {} ORDER BY created_at DESC LIMIT 200",
            if include_ended {
                ""
            } else {
                "AND state NOT IN ('gone', 'failed')"
            }
        )))
        .bind(owner_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(machine_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn machine(&self, owner_id: &str, id: &str) -> ApiResult<Option<MachineRow>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE owner_id = $1 AND id = $2"
        )))
        .bind(owner_id)
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(machine_row).transpose()?)
    }

    pub async fn set_machine_provider(
        &self,
        id: &str,
        provider_id: &str,
        address: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE machines SET provider_id = $1, address = $2, state = 'booting' WHERE id = $3",
        )
        .bind(provider_id)
        .bind(address)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_machine_state(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "UPDATE machines SET state = $1, reason = COALESCE($2, reason),
                ready_at = CASE WHEN $1 = 'ready' AND ready_at IS NULL THEN $3 ELSE ready_at END,
                ended_at = CASE WHEN $1 IN ('gone', 'failed') AND ended_at IS NULL THEN $3 ELSE ended_at END
             WHERE id = $4",
        )
        .bind(state)
        .bind(reason)
        .bind(now)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_machine_cost(&self, id: &str, cost_micros: i64) -> ApiResult<()> {
        sqlx::query("UPDATE machines SET cost_micros = $1 WHERE id = $2")
            .bind(cost_micros)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn fleet_event(
        &self,
        owner_id: &str,
        kind: &str,
        machine: Option<&str>,
        detail: &str,
    ) -> ApiResult<()> {
        sqlx::query("INSERT INTO fleet_events (id, owner_id, at, kind, machine, detail) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(new_id("fev"))
            .bind(owner_id)
            .bind(now_ms())
            .bind(kind)
            .bind(machine)
            .bind(detail)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn fleet_events(&self, owner_id: &str, limit: i64) -> ApiResult<Vec<FleetEvent>> {
        let rows = sqlx::query(
            "SELECT at, kind, machine, detail FROM fleet_events WHERE owner_id = $1 ORDER BY at DESC LIMIT $2",
        )
        .bind(owner_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(FleetEvent {
                    at: rfc3339(r.try_get("at")?),
                    kind: r.try_get("kind")?,
                    machine: r.try_get("machine")?,
                    detail: r.try_get("detail")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// What finished machines cost since `since_ms`, plus running ones so far.
    pub async fn fleet_cost_since(&self, owner_id: &str, since_ms: i64) -> ApiResult<i64> {
        let total: Option<i64> = sqlx::query_scalar(
            // SUM is NUMERIC on PostgreSQL: cast it back to an integer.
            "SELECT CAST(COALESCE(SUM(cost_micros), 0) AS BIGINT) FROM machines WHERE owner_id = $1 AND (ended_at IS NULL OR ended_at >= $2)",
        )
        .bind(owner_id)
        .bind(since_ms)
        .fetch_one(self.pool())
        .await?;
        Ok(total.unwrap_or(0))
    }

    /// When runs of an owner's repositories were queued in the last hour.
    pub async fn recent_arrivals(&self, owner_id: &str, since_ms: i64) -> ApiResult<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT c.created_at FROM check_runs c JOIN repos r ON r.id = c.repo_id
             WHERE r.owner_id = $1 AND c.created_at >= $2 AND c.reused_from IS NULL",
        )
        .bind(owner_id)
        .bind(since_ms)
        .fetch_all(self.pool())
        .await?)
    }

    /// Runs of an owner's repositories since `since_ms`, for simulation:
    /// `(queued, started, finished, labels)`, without reused or skipped ones.
    #[allow(clippy::type_complexity)]
    pub async fn owner_history(
        &self,
        owner_id: &str,
        since_ms: i64,
    ) -> ApiResult<Vec<(i64, Option<i64>, Option<i64>, Vec<String>)>> {
        let rows = sqlx::query(
            "SELECT c.created_at, c.started_at, c.finished_at, c.labels FROM check_runs c
             JOIN repos r ON r.id = c.repo_id
             WHERE r.owner_id = $1 AND c.created_at >= $2 AND c.reused_from IS NULL
               AND c.state IN ('success', 'failure', 'error') AND c.kind = 'command'
             ORDER BY c.created_at LIMIT 20000",
        )
        .bind(owner_id)
        .bind(since_ms)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get("created_at")?,
                    r.try_get("started_at")?,
                    r.try_get("finished_at")?,
                    super::json_list(r, "labels")?,
                ))
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Checks that ran on the owner's runners since `since_ms`, with the
    /// machine of each runner (for the fleet's timeline).
    pub async fn fleet_runs(&self, owner_id: &str, since_ms: i64) -> ApiResult<Vec<FleetRun>> {
        let rows = sqlx::query(
            "SELECT COALESCE(n.machine, n.name) AS machine, c.name, r.name AS repo_name, a.handle, c.state,
                    c.started_at, c.finished_at
             FROM check_runs c
             JOIN runners n ON n.id = c.runner_id
             JOIN repos r ON r.id = c.repo_id
             JOIN accounts a ON a.id = r.owner_id
             WHERE n.owner_id = $1 AND c.started_at IS NOT NULL
               AND (c.finished_at IS NULL OR c.finished_at >= $2)
             ORDER BY c.started_at LIMIT 500",
        )
        .bind(owner_id)
        .bind(since_ms)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                let started: i64 = r.try_get("started_at")?;
                let finished: Option<i64> = r.try_get("finished_at")?;
                let handle: String = r.try_get("handle")?;
                let repo: String = r.try_get("repo_name")?;
                Ok(FleetRun {
                    machine: r.try_get("machine")?,
                    name: r.try_get("name")?,
                    repo: format!("{handle}/{repo}"),
                    state: r.try_get("state")?,
                    started_at: super::rfc3339(started),
                    finished_at: finished.map(super::rfc3339),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Median seconds of each check name across an owner's repositories.
    pub async fn typical_durations(
        &self,
        owner_id: &str,
    ) -> ApiResult<std::collections::HashMap<String, f64>> {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT c.name, c.started_at, c.finished_at FROM check_runs c JOIN repos r ON r.id = c.repo_id
             WHERE r.owner_id = $1 AND c.state IN ('success', 'failure') AND c.started_at IS NOT NULL
               AND c.finished_at IS NOT NULL AND c.reused_from IS NULL
             ORDER BY c.created_at DESC LIMIT 2000",
        )
        .bind(owner_id)
        .fetch_all(self.pool())
        .await?;
        let mut by: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();
        for (name, start, end) in rows {
            let base = name.split(" (").next().unwrap_or(&name).to_owned();
            #[allow(clippy::cast_precision_loss)]
            by.entry(base)
                .or_default()
                .push((end - start) as f64 / 1000.0);
        }
        Ok(by
            .into_iter()
            .map(|(k, mut v)| {
                v.sort_by(f64::total_cmp);
                let median = v[v.len() / 2];
                (k, median)
            })
            .collect())
    }
}
