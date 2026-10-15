//! The transparency log: an append-only, hash-chained record per repository
//! of everything that changes what the repository is (bookmark moves, merges,
//! access and settings). Each entry's hash covers the previous one, so
//! rewriting history anywhere changes every hash after it; the server signs
//! the head for anyone to check (`hb verify-log`).

use serde::Serialize;
use sqlx::any::Any;
use sqlx::{Row, Transaction};

use super::{Db, Dialect, now_ms, rfc3339};
use crate::error::ApiResult;

pub use hebra_core::transparency::{GENESIS, entry_hash};

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub seq: i64,
    pub at: String,
    /// Exactly what the hash covers.
    pub at_ms: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
}

impl Db {
    /// Appends an entry inside `tx` (so it commits with the change it
    /// records). Entries of one repository are serialised by a row lock
    /// (PostgreSQL) or SQLite's single writer.
    pub(crate) async fn append_entry(
        &self,
        tx: &mut Transaction<'_, Any>,
        repo_id: &str,
        kind: &str,
        payload: &serde_json::Value,
    ) -> ApiResult<()> {
        if self.dialect() == Dialect::Postgres {
            sqlx::query("SELECT id FROM repos WHERE id = $1 FOR UPDATE")
                .bind(repo_id)
                .execute(&mut **tx)
                .await?;
        }
        let last: Option<(i64, String)> = sqlx::query_as(
            "SELECT seq, hash FROM log_entries WHERE repo_id = $1 ORDER BY seq DESC LIMIT 1",
        )
        .bind(repo_id)
        .fetch_optional(&mut **tx)
        .await?;
        let (seq, prev) = last.map_or((1, GENESIS.to_owned()), |(s, h)| (s + 1, h));
        let at = now_ms();
        let payload = payload.to_string();
        let hash = entry_hash(&prev, seq, at, kind, &payload);
        sqlx::query(
            "INSERT INTO log_entries (repo_id, seq, at, kind, payload, prev_hash, hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(repo_id)
        .bind(seq)
        .bind(at)
        .bind(kind)
        .bind(&payload)
        .bind(&prev)
        .bind(&hash)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Appends an entry on its own.
    pub async fn log_event(
        &self,
        repo_id: &str,
        kind: &str,
        payload: &serde_json::Value,
    ) -> ApiResult<()> {
        let mut tx = self.pool().begin().await?;
        self.append_entry(&mut tx, repo_id, kind, payload).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Entries after `after`, oldest first (up to `limit`).
    pub async fn log_entries(
        &self,
        repo_id: &str,
        after: i64,
        limit: i64,
    ) -> ApiResult<Vec<LogEntry>> {
        let rows = sqlx::query(
            "SELECT seq, at, kind, payload, prev_hash, hash FROM log_entries
             WHERE repo_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
        )
        .bind(repo_id)
        .bind(after)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                let at_ms: i64 = r.try_get("at")?;
                let payload: String = r.try_get("payload")?;
                Ok(LogEntry {
                    seq: r.try_get("seq")?,
                    at: rfc3339(at_ms),
                    at_ms,
                    kind: r.try_get("kind")?,
                    payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
                    prev_hash: r.try_get("prev_hash")?,
                    hash: r.try_get("hash")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// The newest entry (sequence number and hash).
    pub async fn log_head(&self, repo_id: &str) -> ApiResult<Option<(i64, String)>> {
        Ok(sqlx::query_as(
            "SELECT seq, hash FROM log_entries WHERE repo_id = $1 ORDER BY seq DESC LIMIT 1",
        )
        .bind(repo_id)
        .fetch_optional(self.pool())
        .await?)
    }
}
