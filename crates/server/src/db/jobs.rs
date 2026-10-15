//! Singleton jobs (leases) and the merge queue's batches.

use serde::Serialize;
use sqlx::Row;

use super::{Db, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

/// Reviews a merge batch may take at most.
pub const MAX_BATCH: i64 = 8;

#[derive(Debug, Clone, Serialize)]
pub struct MergeBatch {
    pub id: String,
    #[serde(skip)]
    pub repo_id: String,
    pub target: String,
    pub base_commit: String,
    pub batch_commit: String,
    /// Review numbers, in merge order.
    pub reviews: Vec<i64>,
    pub state: String,
    pub created_at: String,
    pub detail: Option<String>,
}

impl Db {
    /// Takes or renews the lease on a job for `holder`. False if another
    /// replica holds it.
    pub async fn acquire_lease(&self, name: &str, holder: &str, ttl_ms: i64) -> ApiResult<bool> {
        let now = now_ms();
        let taken = sqlx::query(
            "INSERT INTO leases (name, holder, until_ms) VALUES ($1, $2, $3)
             ON CONFLICT (name) DO UPDATE SET holder = excluded.holder, until_ms = excluded.until_ms
             WHERE leases.until_ms < $4 OR leases.holder = $5",
        )
        .bind(name)
        .bind(holder)
        .bind(now + ttl_ms)
        .bind(now)
        .bind(holder)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(taken == 1)
    }

    /// Targets with queued reviews: (repo id, target).
    pub async fn queued_targets(&self) -> ApiResult<Vec<(String, String)>> {
        Ok(
            sqlx::query_as("SELECT DISTINCT repo_id, target FROM reviews WHERE state = 'queued'")
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Queued reviews for a target, oldest first.
    pub async fn queued_reviews(
        &self,
        repo_id: &str,
        target: &str,
        limit: i64,
    ) -> ApiResult<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT number FROM reviews WHERE repo_id = $1 AND target = $2 AND state = 'queued'
             ORDER BY updated_at, number LIMIT $3",
        )
        .bind(repo_id)
        .bind(target)
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn batch_limit(&self, repo_id: &str, target: &str) -> ApiResult<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT batch_limit FROM merge_queue_limits WHERE repo_id = $1 AND target = $2",
        )
        .bind(repo_id)
        .bind(target)
        .fetch_optional(self.pool())
        .await?
        .unwrap_or(MAX_BATCH))
    }

    pub async fn set_batch_limit(&self, repo_id: &str, target: &str, limit: i64) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO merge_queue_limits (repo_id, target, batch_limit) VALUES ($1, $2, $3)
             ON CONFLICT (repo_id, target) DO UPDATE SET batch_limit = excluded.batch_limit",
        )
        .bind(repo_id)
        .bind(target)
        .bind(limit.clamp(1, MAX_BATCH))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn create_batch(
        &self,
        repo_id: &str,
        target: &str,
        base: &str,
        commit: &str,
        reviews: &[i64],
    ) -> ApiResult<String> {
        let id = new_id("mq");
        sqlx::query(
            "INSERT INTO merge_batches (id, repo_id, target, base_commit, batch_commit, reviews, state, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'testing', $7)",
        )
        .bind(&id)
        .bind(repo_id)
        .bind(target)
        .bind(base)
        .bind(commit)
        .bind(serde_json::to_string(reviews)?)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    /// The batch being tested for a target, if any.
    pub async fn active_batch(&self, repo_id: &str, target: &str) -> ApiResult<Option<MergeBatch>> {
        let row = sqlx::query(
            "SELECT id, repo_id, target, base_commit, batch_commit, reviews, state, created_at, detail
             FROM merge_batches WHERE repo_id = $1 AND target = $2 AND state = 'testing'",
        )
        .bind(repo_id)
        .bind(target)
        .fetch_optional(self.pool())
        .await?;
        row.map(|r| {
            let reviews: String = r.try_get("reviews")?;
            Ok(MergeBatch {
                id: r.try_get("id")?,
                repo_id: r.try_get("repo_id")?,
                target: r.try_get("target")?,
                base_commit: r.try_get("base_commit")?,
                batch_commit: r.try_get("batch_commit")?,
                reviews: serde_json::from_str(&reviews).unwrap_or_default(),
                state: r.try_get("state")?,
                created_at: rfc3339(r.try_get("created_at")?),
                detail: r.try_get("detail")?,
            })
        })
        .transpose()
        .map_err(|e: sqlx::Error| e.into())
    }

    pub async fn finish_batch(&self, id: &str, state: &str, detail: &str) -> ApiResult<()> {
        sqlx::query("UPDATE merge_batches SET state = $1, detail = $2, finished_at = $3 WHERE id = $4 AND state = 'testing'")
            .bind(state)
            .bind(detail)
            .bind(now_ms())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Batches of a repository, newest first.
    pub async fn batches(&self, repo_id: &str, limit: i64) -> ApiResult<Vec<MergeBatch>> {
        let rows = sqlx::query(
            "SELECT id, repo_id, target, base_commit, batch_commit, reviews, state, created_at, detail
             FROM merge_batches WHERE repo_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(repo_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                let reviews: String = r.try_get("reviews")?;
                Ok(MergeBatch {
                    id: r.try_get("id")?,
                    repo_id: r.try_get("repo_id")?,
                    target: r.try_get("target")?,
                    base_commit: r.try_get("base_commit")?,
                    batch_commit: r.try_get("batch_commit")?,
                    reviews: serde_json::from_str(&reviews).unwrap_or_default(),
                    state: r.try_get("state")?,
                    created_at: rfc3339(r.try_get("created_at")?),
                    detail: r.try_get("detail")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Every owner with repositories (for storage metering).
    pub async fn repo_owners(&self) -> ApiResult<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT DISTINCT owner_id FROM repos")
            .fetch_all(self.pool())
            .await?)
    }

    /// Ids of an owner's repositories.
    pub async fn repo_ids_of(&self, owner_id: &str) -> ApiResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT id FROM repos WHERE owner_id = $1")
                .bind(owner_id)
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Records today's storage of an owner (GB, rounded up) once per day.
    pub async fn record_storage(&self, owner_id: &str, gigabytes: i64) -> ApiResult<()> {
        let day = jiff::Timestamp::now().strftime("%Y-%m-%d").to_string();
        let reference = format!("storage:{day}");
        let seen: Option<String> = sqlx::query_scalar(
            "SELECT id FROM usage_events WHERE owner_id = $1 AND reference = $2 LIMIT 1",
        )
        .bind(owner_id)
        .bind(&reference)
        .fetch_optional(self.pool())
        .await?;
        if seen.is_none() {
            self.record_usage(owner_id, "storage_gb_days", gigabytes, &reference)
                .await?;
        }
        Ok(())
    }
}
