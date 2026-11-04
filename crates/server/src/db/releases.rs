//! Releases with their files, and webhooks with their deliveries.

use serde::Serialize;
use sqlx::Row;

use super::{Db, json_list, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub id: String,
    pub tag: String,
    pub commit: String,
    pub title: String,
    pub notes: String,
    pub draft: bool,
    pub prerelease: bool,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseAsset {
    pub id: String,
    pub name: String,
    pub size: i64,
    pub digest: String,
    pub content_type: String,
    pub downloads: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Webhook {
    pub id: String,
    #[serde(skip)]
    pub repo_id: String,
    pub url: String,
    #[serde(skip)]
    pub secret: String,
    pub events: Vec<String>,
    pub active: bool,
    pub created_by: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Delivery {
    pub id: String,
    pub event: String,
    pub status: Option<i64>,
    pub error: Option<String>,
    pub duration_ms: i64,
    pub attempt: i64,
    pub payload: serde_json::Value,
    pub at: String,
}

pub struct NewRelease<'a> {
    pub repo_id: &'a str,
    pub tag: &'a str,
    pub commit: &'a str,
    pub title: &'a str,
    pub notes: &'a str,
    pub draft: bool,
    pub prerelease: bool,
    pub by: &'a str,
}

fn release_row(r: &sqlx::any::AnyRow) -> Result<Release, sqlx::Error> {
    Ok(Release {
        id: r.try_get("id")?,
        tag: r.try_get("tag")?,
        commit: r.try_get("commit_id")?,
        title: r.try_get("title")?,
        notes: r.try_get("notes")?,
        draft: r.try_get::<i64, _>("draft")? != 0,
        prerelease: r.try_get::<i64, _>("prerelease")? != 0,
        created_by: r.try_get("created_by")?,
        created_at: rfc3339(r.try_get("created_at")?),
        updated_at: rfc3339(r.try_get("updated_at")?),
    })
}

fn hook_row(r: &sqlx::any::AnyRow) -> Result<Webhook, sqlx::Error> {
    Ok(Webhook {
        id: r.try_get("id")?,
        repo_id: r.try_get("repo_id")?,
        url: r.try_get("url")?,
        secret: r.try_get("secret")?,
        events: json_list(r, "events")?,
        active: r.try_get::<i64, _>("active")? != 0,
        created_by: r.try_get("created_by")?,
        created_at: rfc3339(r.try_get("created_at")?),
    })
}

const RELEASE: &str = "SELECT id, tag, commit_id, title, notes, draft, prerelease, created_by, created_at, updated_at FROM releases";
const HOOK: &str =
    "SELECT id, repo_id, url, secret, events, active, created_by, created_at FROM webhooks";

impl Db {
    pub async fn create_release(&self, r: &NewRelease<'_>) -> ApiResult<String> {
        let id = new_id("rel");
        let now = now_ms();
        sqlx::query(
            "INSERT INTO releases (id, repo_id, tag, commit_id, title, notes, draft, prerelease, created_by, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
        )
        .bind(&id)
        .bind(r.repo_id)
        .bind(r.tag)
        .bind(r.commit)
        .bind(r.title)
        .bind(r.notes)
        .bind(i64::from(r.draft))
        .bind(i64::from(r.prerelease))
        .bind(r.by)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn releases(&self, repo_id: &str) -> ApiResult<Vec<Release>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{RELEASE} WHERE repo_id = $1 ORDER BY created_at DESC LIMIT 200"
        )))
        .bind(repo_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(release_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn release(&self, repo_id: &str, tag: &str) -> ApiResult<Option<Release>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{RELEASE} WHERE repo_id = $1 AND tag = $2"
        )))
        .bind(repo_id)
        .bind(tag)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(release_row).transpose()?)
    }

    pub async fn update_release(
        &self,
        id: &str,
        title: Option<&str>,
        notes: Option<&str>,
        draft: Option<bool>,
        prerelease: Option<bool>,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE releases SET title = COALESCE($1, title), notes = COALESCE($2, notes),
                draft = COALESCE($3, draft), prerelease = COALESCE($4, prerelease), updated_at = $5
             WHERE id = $6",
        )
        .bind(title)
        .bind(notes)
        .bind(draft.map(i64::from))
        .bind(prerelease.map(i64::from))
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_release(&self, id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM releases WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn add_asset(
        &self,
        release_id: &str,
        name: &str,
        size: u64,
        digest: &str,
        content_type: &str,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO release_assets (id, release_id, name, size, digest, content_type, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (release_id, name) DO UPDATE SET size = excluded.size, digest = excluded.digest,
                content_type = excluded.content_type, created_at = excluded.created_at",
        )
        .bind(new_id("ast"))
        .bind(release_id)
        .bind(name)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .bind(digest)
        .bind(content_type)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn assets(&self, release_id: &str) -> ApiResult<Vec<ReleaseAsset>> {
        let rows = sqlx::query(
            "SELECT id, name, size, digest, content_type, downloads, created_at FROM release_assets
             WHERE release_id = $1 ORDER BY name",
        )
        .bind(release_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(ReleaseAsset {
                    id: r.try_get("id")?,
                    name: r.try_get("name")?,
                    size: r.try_get("size")?,
                    digest: r.try_get("digest")?,
                    content_type: r.try_get("content_type")?,
                    downloads: r.try_get("downloads")?,
                    created_at: rfc3339(r.try_get("created_at")?),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    pub async fn count_download(&self, release_id: &str, name: &str) -> ApiResult<()> {
        sqlx::query("UPDATE release_assets SET downloads = downloads + 1 WHERE release_id = $1 AND name = $2")
            .bind(release_id)
            .bind(name)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_asset(&self, release_id: &str, name: &str) -> ApiResult<bool> {
        Ok(
            sqlx::query("DELETE FROM release_assets WHERE release_id = $1 AND name = $2")
                .bind(release_id)
                .bind(name)
                .execute(self.pool())
                .await?
                .rows_affected()
                == 1,
        )
    }

    // -------------------------------------------------------------- webhooks

    pub async fn create_webhook(
        &self,
        repo_id: &str,
        url: &str,
        sealed: &str,
        events: &[String],
        by: &str,
    ) -> ApiResult<String> {
        let id = new_id("hook");
        sqlx::query(
            "INSERT INTO webhooks (id, repo_id, url, secret, events, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(repo_id)
        .bind(url)
        .bind(sealed)
        .bind(serde_json::to_string(events)?)
        .bind(by)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn seal_webhook(&self, id: &str, sealed: &str) -> ApiResult<()> {
        sqlx::query("UPDATE webhooks SET secret = $1 WHERE id = $2")
            .bind(sealed)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn webhooks(&self, repo_id: &str) -> ApiResult<Vec<Webhook>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{HOOK} WHERE repo_id = $1 ORDER BY created_at"
        )))
        .bind(repo_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(hook_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn webhook(&self, id: &str) -> ApiResult<Option<Webhook>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{HOOK} WHERE id = $1")))
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(hook_row).transpose()?)
    }

    pub async fn set_webhook(
        &self,
        id: &str,
        active: Option<bool>,
        events: Option<&[String]>,
    ) -> ApiResult<()> {
        sqlx::query("UPDATE webhooks SET active = COALESCE($1, active), events = COALESCE($2, events) WHERE id = $3")
            .bind(active.map(i64::from))
            .bind(events.map(|e| serde_json::to_string(e).unwrap_or_default()))
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_webhook(&self, id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM webhooks WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_delivery(
        &self,
        hook_id: &str,
        event: &str,
        status: Option<i64>,
        error: Option<&str>,
        duration_ms: i64,
        attempt: i64,
        payload: &serde_json::Value,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO webhook_deliveries (id, hook_id, event, status, error, duration_ms, attempt, payload, at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(new_id("dlv"))
        .bind(hook_id)
        .bind(event)
        .bind(status)
        .bind(error)
        .bind(duration_ms)
        .bind(attempt)
        .bind(payload.to_string())
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        // Only the recent past is kept.
        sqlx::query(
            "DELETE FROM webhook_deliveries WHERE hook_id = $1 AND id NOT IN
               (SELECT id FROM webhook_deliveries WHERE hook_id = $1 ORDER BY at DESC LIMIT 100)",
        )
        .bind(hook_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn deliveries(&self, hook_id: &str) -> ApiResult<Vec<Delivery>> {
        let rows = sqlx::query(
            "SELECT id, event, status, error, duration_ms, attempt, payload, at FROM webhook_deliveries
             WHERE hook_id = $1 ORDER BY at DESC LIMIT 50",
        )
        .bind(hook_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                let payload: String = r.try_get("payload")?;
                Ok(Delivery {
                    id: r.try_get("id")?,
                    event: r.try_get("event")?,
                    status: r.try_get("status")?,
                    error: r.try_get("error")?,
                    duration_ms: r.try_get("duration_ms")?,
                    attempt: r.try_get("attempt")?,
                    payload: serde_json::from_str(&payload).unwrap_or_default(),
                    at: rfc3339(r.try_get("at")?),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }
}
