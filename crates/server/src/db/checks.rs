//! Runners and check runs: a work queue with leases. A runner claims the
//! oldest queued run it may take, renews the lease while it works, and
//! reports the result; a run whose lease expires is queued again.

use serde::Serialize;
use sqlx::Row;
use sqlx::any::AnyRow;

use super::{Db, Dialect, json_list, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

/// Attempts before a run whose runner keeps disappearing is an error.
pub const MAX_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, Serialize)]
pub struct Runner {
    pub id: String,
    pub owner_id: String,
    pub account_id: String,
    pub name: String,
    pub labels: Vec<String>,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub version: Option<String>,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckRun {
    pub id: String,
    #[serde(skip)]
    pub repo_id: String,
    pub commit: String,
    pub name: String,
    pub kind: String,
    /// What to run: `{ "run": "...", "timeout_secs": n }` for commands,
    /// `{ "contract": "...", ... }` for agent verification.
    pub spec: serde_json::Value,
    pub labels: Vec<String>,
    pub cause: String,
    #[serde(skip)]
    pub review_id: Option<String>,
    pub required: bool,
    pub state: String,
    pub runner_id: Option<String>,
    pub attempt: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub exit_code: Option<i64>,
    pub summary: Option<String>,
    pub log_bytes: i64,
    pub credits: i64,
}

pub struct NewCheckRun<'a> {
    pub repo_id: &'a str,
    pub commit: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub spec: &'a serde_json::Value,
    pub labels: &'a [String],
    pub cause: &'a str,
    pub review_id: Option<&'a str>,
    pub required: bool,
    /// `queued`, or `waiting` when a maintainer must approve it first.
    pub state: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogChunk {
    pub seq: i64,
    pub text: String,
}

macro_rules! run_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, repo_id, commit_id, name, kind, spec, labels, cause, review_id, required, state, runner_id,
                    attempt, created_at, started_at, finished_at, exit_code, summary, log_bytes, credits
             FROM check_runs ",
            $tail
        )
    };
}

fn run_row(r: &AnyRow) -> Result<CheckRun, sqlx::Error> {
    let spec: String = r.try_get("spec")?;
    Ok(CheckRun {
        id: r.try_get("id")?,
        repo_id: r.try_get("repo_id")?,
        commit: r.try_get("commit_id")?,
        name: r.try_get("name")?,
        kind: r.try_get("kind")?,
        spec: serde_json::from_str(&spec).map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
        labels: json_list(r, "labels")?,
        cause: r.try_get("cause")?,
        review_id: r.try_get("review_id")?,
        required: r.try_get::<i64, _>("required")? != 0,
        state: r.try_get("state")?,
        runner_id: r.try_get("runner_id")?,
        attempt: r.try_get("attempt")?,
        created_at: rfc3339(r.try_get("created_at")?),
        started_at: r.try_get::<Option<i64>, _>("started_at")?.map(rfc3339),
        finished_at: r.try_get::<Option<i64>, _>("finished_at")?.map(rfc3339),
        exit_code: r.try_get("exit_code")?,
        summary: r.try_get("summary")?,
        log_bytes: r.try_get("log_bytes")?,
        credits: r.try_get("credits")?,
    })
}

/// A runner counts as online if it was seen this recently.
const ONLINE_MS: i64 = 90_000;

macro_rules! runner_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, owner_id, account_id, name, labels, created_at, last_seen_at, version FROM runners ",
            $tail
        )
    };
}

fn runner_row(r: &AnyRow, now: i64) -> Result<Runner, sqlx::Error> {
    let last_seen: Option<i64> = r.try_get("last_seen_at")?;
    Ok(Runner {
        id: r.try_get("id")?,
        owner_id: r.try_get("owner_id")?,
        account_id: r.try_get("account_id")?,
        name: r.try_get("name")?,
        labels: json_list(r, "labels")?,
        created_at: rfc3339(r.try_get("created_at")?),
        last_seen_at: last_seen.map(rfc3339),
        version: r.try_get("version")?,
        online: last_seen.is_some_and(|s| now - s < ONLINE_MS),
    })
}

impl Db {
    // --------------------------------------------------------------- runners

    pub async fn create_runner(
        &self,
        owner_id: &str,
        account_id: &str,
        name: &str,
        labels: &[String],
    ) -> ApiResult<String> {
        let id = new_id("rnr");
        sqlx::query(
            "INSERT INTO runners (id, owner_id, account_id, name, labels, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(owner_id)
        .bind(account_id)
        .bind(name)
        .bind(serde_json::to_string(labels)?)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn runners(&self, owner_id: &str) -> ApiResult<Vec<Runner>> {
        let rows = sqlx::query(runner_select!("WHERE owner_id = $1 ORDER BY name"))
            .bind(owner_id)
            .fetch_all(self.pool())
            .await?;
        let now = now_ms();
        Ok(rows
            .iter()
            .map(|r| runner_row(r, now))
            .collect::<Result<_, _>>()?)
    }

    pub async fn runner_by_account(&self, account_id: &str) -> ApiResult<Option<Runner>> {
        let row = sqlx::query(runner_select!("WHERE account_id = $1"))
            .bind(account_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(|r| runner_row(r, now_ms())).transpose()?)
    }

    pub async fn runner_seen(&self, runner_id: &str, version: &str) -> ApiResult<()> {
        sqlx::query("UPDATE runners SET last_seen_at = $1, version = $2 WHERE id = $3")
            .bind(now_ms())
            .bind(version)
            .bind(runner_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_runner(&self, runner_id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM runners WHERE id = $1")
            .bind(runner_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------------ runs

    pub async fn create_run(&self, r: &NewCheckRun<'_>) -> ApiResult<String> {
        let id = new_id("chk");
        sqlx::query(
            "INSERT INTO check_runs (id, repo_id, commit_id, name, kind, spec, labels, cause, review_id, required,
                                     state, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(&id)
        .bind(r.repo_id)
        .bind(r.commit)
        .bind(r.name)
        .bind(r.kind)
        .bind(r.spec.to_string())
        .bind(serde_json::to_string(r.labels)?)
        .bind(r.cause)
        .bind(r.review_id)
        .bind(i64::from(r.required))
        .bind(r.state)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn run(&self, id: &str) -> ApiResult<Option<CheckRun>> {
        let row = sqlx::query(run_select!("WHERE id = $1"))
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(run_row).transpose()?)
    }

    /// The latest run of each check on a commit.
    pub async fn runs_for_commit(&self, repo_id: &str, commit: &str) -> ApiResult<Vec<CheckRun>> {
        let rows = sqlx::query(run_select!(
            "WHERE repo_id = $1 AND commit_id = $2 ORDER BY created_at DESC"
        ))
        .bind(repo_id)
        .bind(commit)
        .fetch_all(self.pool())
        .await?;
        let mut latest: Vec<CheckRun> = Vec::new();
        for row in &rows {
            let run = run_row(row)?;
            if !latest.iter().any(|r| r.name == run.name) {
                latest.push(run);
            }
        }
        latest.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(latest)
    }

    pub async fn recent_runs(&self, repo_id: &str, limit: i64) -> ApiResult<Vec<CheckRun>> {
        let rows = sqlx::query(run_select!(
            "WHERE repo_id = $1 ORDER BY created_at DESC LIMIT $2"
        ))
        .bind(repo_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.iter().map(run_row).collect::<Result<_, _>>()?)
    }

    /// Moves `waiting` runs of a commit to `queued` (a maintainer approved
    /// running them).
    pub async fn approve_runs(&self, repo_id: &str, commit: &str) -> ApiResult<u64> {
        Ok(sqlx::query("UPDATE check_runs SET state = 'queued' WHERE repo_id = $1 AND commit_id = $2 AND state = 'waiting'")
            .bind(repo_id)
            .bind(commit)
            .execute(self.pool())
            .await?
            .rows_affected())
    }

    /// Leases the oldest queued run the runner may take: in a repository of
    /// its owner, with every label of the run among the runner's. Returns
    /// the run, now `running`.
    pub async fn claim_run(&self, runner: &Runner, lease_ms: i64) -> ApiResult<Option<CheckRun>> {
        let now = now_ms();
        let mut tx = self.pool().begin().await?;
        let lock = match self.dialect() {
            Dialect::Postgres => " FOR UPDATE OF c SKIP LOCKED",
            Dialect::Sqlite => "",
        };
        let candidates: Vec<(String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT c.id, c.labels FROM check_runs c JOIN repos r ON r.id = c.repo_id
             WHERE r.owner_id = $1 AND c.state = 'queued'
             ORDER BY c.created_at LIMIT 50{lock}"
        )))
        .bind(&runner.owner_id)
        .fetch_all(&mut *tx)
        .await?;
        let chosen = candidates.into_iter().find(|(_, labels)| {
            serde_json::from_str::<Vec<String>>(labels)
                .is_ok_and(|need| need.iter().all(|l| runner.labels.contains(l)))
        });
        let Some((id, _)) = chosen else {
            tx.commit().await?;
            return Ok(None);
        };
        let taken = sqlx::query(
            "UPDATE check_runs SET state = 'running', runner_id = $1, attempt = attempt + 1, lease_until = $2,
                                   started_at = $3
             WHERE id = $4 AND state = 'queued'",
        )
        .bind(&runner.id)
        .bind(now + lease_ms)
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if taken == 0 {
            return Ok(None);
        }
        self.run(&id).await
    }

    /// Renews the lease of a running run held by `runner_id`. False if the
    /// run is no longer this runner's (expired and re-queued, or cancelled).
    pub async fn renew_run(&self, run_id: &str, runner_id: &str, lease_ms: i64) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE check_runs SET lease_until = $1 WHERE id = $2 AND runner_id = $3 AND state = 'running'",
        )
        .bind(now_ms() + lease_ms)
        .bind(run_id)
        .bind(runner_id)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn append_log(&self, run_id: &str, text: &str, max_bytes: i64) -> ApiResult<bool> {
        let used: i64 = sqlx::query_scalar("SELECT log_bytes FROM check_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(self.pool())
            .await?;
        if used >= max_bytes {
            return Ok(false);
        }
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM check_logs WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(self.pool())
        .await?;
        let mut tx = self.pool().begin().await?;
        sqlx::query("INSERT INTO check_logs (run_id, seq, chunk) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(run_id)
            .bind(seq)
            .bind(text)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE check_runs SET log_bytes = log_bytes + $1 WHERE id = $2")
            .bind(i64::try_from(text.len()).unwrap_or(i64::MAX))
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn log(&self, run_id: &str, after: i64) -> ApiResult<Vec<LogChunk>> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT seq, chunk FROM check_logs WHERE run_id = $1 AND seq > $2 ORDER BY seq",
        )
        .bind(run_id)
        .bind(after)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(seq, text)| LogChunk { seq, text })
            .collect())
    }

    /// Records the result of a running run held by `runner_id`.
    pub async fn finish_run(
        &self,
        run_id: &str,
        runner_id: Option<&str>,
        state: &str,
        exit_code: Option<i64>,
        summary: &str,
        credits: i64,
    ) -> ApiResult<bool> {
        let rows = match runner_id {
            Some(runner) => sqlx::query(
                "UPDATE check_runs SET state = $1, exit_code = $2, summary = $3, credits = credits + $4,
                                       finished_at = $5, lease_until = NULL
                 WHERE id = $6 AND runner_id = $7 AND state = 'running'",
            )
            .bind(state)
            .bind(exit_code)
            .bind(summary)
            .bind(credits)
            .bind(now_ms())
            .bind(run_id)
            .bind(runner)
            .execute(self.pool())
            .await?,
            None => sqlx::query(
                "UPDATE check_runs SET state = $1, exit_code = $2, summary = $3, credits = credits + $4,
                                       finished_at = $5, lease_until = NULL
                 WHERE id = $6 AND state IN ('waiting', 'queued', 'running')",
            )
            .bind(state)
            .bind(exit_code)
            .bind(summary)
            .bind(credits)
            .bind(now_ms())
            .bind(run_id)
            .execute(self.pool())
            .await?,
        };
        Ok(rows.rows_affected() == 1)
    }

    /// Marks earlier runs of a commit (one check, or all) as superseded, so
    /// they can be queued again.
    pub async fn supersede_runs(
        &self,
        repo_id: &str,
        commit: &str,
        name: Option<&str>,
    ) -> ApiResult<u64> {
        let now = now_ms();
        let rows = match name {
            Some(name) => sqlx::query(
                "UPDATE check_runs SET state = 'cancelled', finished_at = COALESCE(finished_at, $1), lease_until = NULL
                 WHERE repo_id = $2 AND commit_id = $3 AND name = $4 AND state != 'cancelled'",
            )
            .bind(now)
            .bind(repo_id)
            .bind(commit)
            .bind(name)
            .execute(self.pool())
            .await?,
            None => sqlx::query(
                "UPDATE check_runs SET state = 'cancelled', finished_at = COALESCE(finished_at, $1), lease_until = NULL
                 WHERE repo_id = $2 AND commit_id = $3 AND state != 'cancelled'",
            )
            .bind(now)
            .bind(repo_id)
            .bind(commit)
            .execute(self.pool())
            .await?,
        };
        Ok(rows.rows_affected())
    }

    pub async fn add_run_credits(&self, run_id: &str, credits: i64) -> ApiResult<()> {
        sqlx::query("UPDATE check_runs SET credits = credits + $1 WHERE id = $2")
            .bind(credits)
            .bind(run_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Re-queues runs whose lease expired, or fails them after too many
    /// attempts. Returns the runs that changed.
    pub async fn expire_runs(&self) -> ApiResult<Vec<CheckRun>> {
        let now = now_ms();
        let expired = sqlx::query(run_select!("WHERE state = 'running' AND lease_until < $1"))
            .bind(now)
            .fetch_all(self.pool())
            .await?;
        let mut changed = Vec::new();
        for row in &expired {
            let run = run_row(row)?;
            let (state, summary) = if run.attempt >= MAX_ATTEMPTS {
                (
                    "error",
                    Some("the runner stopped responding too many times"),
                )
            } else {
                ("queued", None)
            };
            let n = sqlx::query(
                "UPDATE check_runs SET state = $1, runner_id = NULL, lease_until = NULL, summary = $2,
                                       finished_at = $3
                 WHERE id = $4 AND state = 'running' AND lease_until < $5",
            )
            .bind(state)
            .bind(summary)
            .bind((state == "error").then_some(now))
            .bind(&run.id)
            .bind(now)
            .execute(self.pool())
            .await?
            .rows_affected();
            if n == 1 {
                changed.push(run);
            }
        }
        Ok(changed)
    }
}
