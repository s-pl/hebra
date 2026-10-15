//! Metadata in PostgreSQL, shared by every replica. Conditional statements
//! make concurrent writes from different pods safe.

use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};

use super::{
    Move, NewReview, RawEvent, RawPresence, RawToken, RepoSummary, Review, Stale, TokenRow,
};
use crate::auth::TokenInfo;
use crate::error::ApiResult;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repos (
    name TEXT PRIMARY KEY, created_at TEXT NOT NULL, public BOOLEAN NOT NULL DEFAULT TRUE);
CREATE TABLE IF NOT EXISTS bookmarks (
    repo TEXT NOT NULL, name TEXT NOT NULL, target TEXT NOT NULL,
    updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, PRIMARY KEY (repo, name));
CREATE TABLE IF NOT EXISTS protected (
    repo TEXT NOT NULL, pattern TEXT NOT NULL, PRIMARY KEY (repo, pattern));
CREATE TABLE IF NOT EXISTS tokens (
    hash TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
    on_behalf_of TEXT, allow TEXT NOT NULL, admin BOOLEAN NOT NULL, created_at TEXT NOT NULL,
    expires_at BIGINT);
CREATE INDEX IF NOT EXISTS tokens_by_name ON tokens (name);
CREATE TABLE IF NOT EXISTS presence (
    repo TEXT NOT NULL, client TEXT NOT NULL, payload TEXT NOT NULL,
    updated_at BIGINT NOT NULL, PRIMARY KEY (repo, client));
CREATE INDEX IF NOT EXISTS presence_by_age ON presence (updated_at);
CREATE TABLE IF NOT EXISTS events (
    id BIGSERIAL PRIMARY KEY, repo TEXT NOT NULL, at TEXT NOT NULL, actor TEXT NOT NULL,
    kind TEXT NOT NULL, detail TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS events_by_repo ON events (repo, id);
CREATE TABLE IF NOT EXISTS reviews (
    id BIGSERIAL PRIMARY KEY, repo TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL, author TEXT NOT NULL, state TEXT NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, closed_by TEXT, merged_commit TEXT);
CREATE INDEX IF NOT EXISTS reviews_by_repo ON reviews (repo, state);
";

/// Serialises schema creation when many replicas start at once.
const MIGRATION_LOCK: i64 = 0x0068_6562_7261; // "hebra"

macro_rules! review_select {
    ($where:literal) => {
        concat!(
            "SELECT id, source, target, title, body, author, state, created_at, updated_at, closed_by, merged_commit FROM reviews ",
            $where
        )
    };
}

#[derive(Clone)]
pub struct Postgres(PgPool);

fn review_row(r: &PgRow) -> Result<Review, sqlx::Error> {
    Ok(Review {
        id: r.try_get(0)?,
        source: r.try_get(1)?,
        target: r.try_get(2)?,
        title: r.try_get(3)?,
        body: r.try_get(4)?,
        author: r.try_get(5)?,
        state: r.try_get(6)?,
        created_at: r.try_get(7)?,
        updated_at: r.try_get(8)?,
        closed_by: r.try_get(9)?,
        merged_commit: r.try_get(10)?,
    })
}

impl Postgres {
    /// Connects (TLS settings from the URL or PGSSLMODE / PGSSLROOTCERT) and
    /// creates the schema, under an advisory lock.
    pub async fn connect(url: &str, max_connections: u32) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await?;
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK)
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql(SCHEMA).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self(pool))
    }

    pub async fn ping(&self) -> ApiResult<()> {
        sqlx::query("SELECT 1").execute(&self.0).await?;
        Ok(())
    }

    pub async fn create_repo(&self, name: &str, protect: &[String], at: &str) -> ApiResult<bool> {
        let mut tx = self.0.begin().await?;
        let created = sqlx::query(
            "INSERT INTO repos (name, created_at) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(name)
        .bind(at)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        for p in protect {
            sqlx::query(
                "INSERT INTO protected (repo, pattern) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(name)
            .bind(p)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(created)
    }

    pub async fn repo_public(&self, name: &str) -> ApiResult<Option<bool>> {
        Ok(
            sqlx::query_scalar("SELECT public FROM repos WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.0)
                .await?,
        )
    }

    pub async fn public_repos(&self) -> ApiResult<Vec<RepoSummary>> {
        let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(
            "SELECT r.name,
                (SELECT COUNT(*) FROM bookmarks b WHERE b.repo = r.name),
                (SELECT MAX(at) FROM events e WHERE e.repo = r.name)
             FROM repos r WHERE r.public ORDER BY r.name",
        )
        .fetch_all(&self.0)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, bookmarks, last_activity)| RepoSummary {
                name,
                bookmarks,
                last_activity,
            })
            .collect())
    }

    pub async fn bookmarks(&self, repo: &str) -> ApiResult<BTreeMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT name, target FROM bookmarks WHERE repo = $1")
                .bind(repo)
                .fetch_all(&self.0)
                .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn bookmark(&self, repo: &str, name: &str) -> ApiResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT target FROM bookmarks WHERE repo = $1 AND name = $2")
                .bind(repo)
                .bind(name)
                .fetch_optional(&self.0)
                .await?,
        )
    }

    pub async fn protected(&self, repo: &str) -> ApiResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT pattern FROM protected WHERE repo = $1 ORDER BY pattern")
                .bind(repo)
                .fetch_all(&self.0)
                .await?,
        )
    }

    pub async fn cas_bookmark(&self, m: &Move<'_>) -> ApiResult<Result<(), Stale>> {
        let mut tx = self.0.begin().await?;
        let moved = match (m.old, m.new) {
            (None, Some(new)) => sqlx::query(
                "INSERT INTO bookmarks (repo, name, target, updated_at, updated_by)
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
            )
            .bind(m.repo)
            .bind(m.name)
            .bind(new)
            .bind(m.at)
            .bind(m.actor)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            (Some(old), Some(new)) => sqlx::query(
                "UPDATE bookmarks SET target = $3, updated_at = $4, updated_by = $5
                 WHERE repo = $1 AND name = $2 AND target = $6",
            )
            .bind(m.repo)
            .bind(m.name)
            .bind(new)
            .bind(m.at)
            .bind(m.actor)
            .bind(old)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            (Some(old), None) => {
                sqlx::query("DELETE FROM bookmarks WHERE repo = $1 AND name = $2 AND target = $3")
                    .bind(m.repo)
                    .bind(m.name)
                    .bind(old)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
            }
            (None, None) => u64::from(
                sqlx::query("SELECT 1 FROM bookmarks WHERE repo = $1 AND name = $2")
                    .bind(m.repo)
                    .bind(m.name)
                    .fetch_optional(&mut *tx)
                    .await?
                    .is_none(),
            ),
        };
        if moved == 0 {
            let current: Option<String> =
                sqlx::query_scalar("SELECT target FROM bookmarks WHERE repo = $1 AND name = $2")
                    .bind(m.repo)
                    .bind(m.name)
                    .fetch_optional(&mut *tx)
                    .await?;
            return Ok(Err(Stale(current)));
        }
        sqlx::query("INSERT INTO events (repo, at, actor, kind, detail) VALUES ($1, $2, $3, 'bookmark', $4)")
            .bind(m.repo)
            .bind(m.at)
            .bind(m.actor)
            .bind(m.detail())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Ok(()))
    }

    pub async fn add_event(
        &self,
        repo: &str,
        at: &str,
        actor: &str,
        kind: &str,
        detail: &str,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO events (repo, at, actor, kind, detail) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(repo)
        .bind(at)
        .bind(actor)
        .bind(kind)
        .bind(detail)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    pub async fn events(&self, repo: &str, limit: i64) -> ApiResult<Vec<RawEvent>> {
        Ok(sqlx::query_as(
            "SELECT at, actor, kind, detail FROM events WHERE repo = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(repo)
        .bind(limit)
        .fetch_all(&self.0)
        .await?)
    }

    pub async fn insert_token(
        &self,
        hash: &str,
        info: &TokenInfo,
        allow: &str,
        at: &str,
        expires_at: Option<i64>,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO tokens (hash, name, kind, on_behalf_of, allow, admin, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(hash)
        .bind(&info.name)
        .bind(&info.kind)
        .bind(&info.on_behalf_of)
        .bind(allow)
        .bind(info.admin)
        .bind(at)
        .bind(expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    pub async fn token(&self, hash: &str, now: i64) -> ApiResult<Option<RawToken>> {
        Ok(sqlx::query_as(
            "SELECT name, kind, on_behalf_of, allow, admin FROM tokens
             WHERE hash = $1 AND (expires_at IS NULL OR expires_at > $2)",
        )
        .bind(hash)
        .bind(now)
        .fetch_optional(&self.0)
        .await?)
    }

    pub async fn tokens(&self) -> ApiResult<Vec<TokenRow>> {
        let rows: Vec<(String, String, String, bool, String, Option<i64>)> = sqlx::query_as(
            "SELECT name, kind, allow, admin, created_at, expires_at FROM tokens ORDER BY created_at",
        )
        .fetch_all(&self.0)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(name, kind, allow, admin, created_at, expires_at)| TokenRow {
                    name,
                    kind,
                    allow: serde_json::from_str(&allow).unwrap_or_default(),
                    admin,
                    created_at,
                    expires_at,
                },
            )
            .collect())
    }

    pub async fn revoke_tokens(&self, name: &str) -> ApiResult<u64> {
        Ok(sqlx::query("DELETE FROM tokens WHERE name = $1")
            .bind(name)
            .execute(&self.0)
            .await?
            .rows_affected())
    }

    pub async fn put_presence(
        &self,
        repo: &str,
        client: &str,
        payload: &str,
        at: i64,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO presence (repo, client, payload, updated_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (repo, client) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
        )
        .bind(repo)
        .bind(client)
        .bind(payload)
        .bind(at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    pub async fn presence(&self, repo: &str, cutoff: i64) -> ApiResult<Vec<RawPresence>> {
        Ok(sqlx::query_as(
            "SELECT client, payload, updated_at FROM presence
             WHERE repo = $1 AND updated_at > $2 ORDER BY updated_at DESC",
        )
        .bind(repo)
        .bind(cutoff)
        .fetch_all(&self.0)
        .await?)
    }

    pub async fn prune_presence(&self, cutoff: i64) -> ApiResult<u64> {
        Ok(sqlx::query("DELETE FROM presence WHERE updated_at <= $1")
            .bind(cutoff)
            .execute(&self.0)
            .await?
            .rows_affected())
    }

    pub async fn open_review_for(
        &self,
        repo: &str,
        source: &str,
        target: &str,
    ) -> ApiResult<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM reviews WHERE repo = $1 AND source = $2 AND target = $3 AND state = 'open'",
        )
        .bind(repo)
        .bind(source)
        .bind(target)
        .fetch_optional(&self.0)
        .await?)
    }

    pub async fn insert_review(&self, r: &NewReview<'_>) -> ApiResult<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO reviews (repo, source, target, title, body, author, state, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'open', $7, $7) RETURNING id",
        )
        .bind(r.repo)
        .bind(r.source)
        .bind(r.target)
        .bind(r.title)
        .bind(r.body)
        .bind(r.author)
        .bind(r.at)
        .fetch_one(&self.0)
        .await?)
    }

    pub async fn review(&self, repo: &str, id: i64) -> ApiResult<Option<Review>> {
        let row = sqlx::query(review_select!("WHERE repo = $1 AND id = $2"))
            .bind(repo)
            .bind(id)
            .fetch_optional(&self.0)
            .await?;
        Ok(row.as_ref().map(review_row).transpose()?)
    }

    pub async fn reviews(&self, repo: &str, state: &str) -> ApiResult<Vec<Review>> {
        let rows = sqlx::query(review_select!(
            "WHERE repo = $1 AND ($2 = 'all' OR state = $2) ORDER BY id DESC LIMIT 200"
        ))
        .bind(repo)
        .bind(state)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.iter().map(review_row).collect::<Result<_, _>>()?)
    }

    pub async fn finish_review(
        &self,
        repo: &str,
        id: i64,
        state: &str,
        by: &str,
        merged: Option<&str>,
        at: &str,
    ) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE reviews SET state = $1, closed_by = $2, merged_commit = $3, updated_at = $4
             WHERE repo = $5 AND id = $6 AND state = 'open'",
        )
        .bind(state)
        .bind(by)
        .bind(merged)
        .bind(at)
        .bind(repo)
        .bind(id)
        .execute(&self.0)
        .await?
        .rows_affected()
            == 1)
    }
}
