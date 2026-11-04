//! Packages (npm and container images), their versions, tags and blobs.

use serde::Serialize;
use sqlx::Row;

use super::{Db, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub id: String,
    #[serde(skip)]
    pub owner_id: String,
    pub kind: String,
    pub name: String,
    pub public: bool,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageVersion {
    pub version: String,
    pub digest: String,
    pub size: i64,
    pub meta: serde_json::Value,
    pub published_by: Option<String>,
    pub created_at: String,
}

fn package_row(r: &sqlx::any::AnyRow) -> Result<Package, sqlx::Error> {
    Ok(Package {
        id: r.try_get("id")?,
        owner_id: r.try_get("owner_id")?,
        kind: r.try_get("kind")?,
        name: r.try_get("name")?,
        public: r.try_get::<i64, _>("public")? != 0,
        description: r.try_get("description")?,
        created_at: rfc3339(r.try_get("created_at")?),
        updated_at: rfc3339(r.try_get("updated_at")?),
    })
}

fn version_row(r: &sqlx::any::AnyRow) -> Result<PackageVersion, sqlx::Error> {
    let meta: String = r.try_get("meta")?;
    Ok(PackageVersion {
        version: r.try_get("version")?,
        digest: r.try_get("digest")?,
        size: r.try_get("size")?,
        meta: serde_json::from_str(&meta).unwrap_or_default(),
        published_by: r.try_get("published_by")?,
        created_at: rfc3339(r.try_get("created_at")?),
    })
}

const PACKAGE: &str =
    "SELECT id, owner_id, kind, name, public, description, created_at, updated_at FROM packages";

impl Db {
    pub async fn package(&self, kind: &str, name: &str) -> ApiResult<Option<Package>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{PACKAGE} WHERE kind = $1 AND name = $2"
        )))
        .bind(kind)
        .bind(name)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(package_row).transpose()?)
    }

    /// The package, created (private) if it is new.
    pub async fn package_or_create(
        &self,
        owner_id: &str,
        kind: &str,
        name: &str,
    ) -> ApiResult<Package> {
        if let Some(p) = self.package(kind, name).await? {
            return Ok(p);
        }
        let now = now_ms();
        sqlx::query(
            "INSERT INTO packages (id, owner_id, kind, name, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5)
             ON CONFLICT (kind, name) DO NOTHING",
        )
        .bind(new_id("pkg"))
        .bind(owner_id)
        .bind(kind)
        .bind(name)
        .bind(now)
        .execute(self.pool())
        .await?;
        self.package(kind, name)
            .await?
            .ok_or_else(|| crate::error::ApiError::internal("packages", "package vanished"))
    }

    pub async fn packages(&self, owner_id: &str) -> ApiResult<Vec<Package>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{PACKAGE} WHERE owner_id = $1 ORDER BY updated_at DESC"
        )))
        .bind(owner_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(package_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn set_package(
        &self,
        id: &str,
        public: Option<bool>,
        description: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE packages SET public = COALESCE($1, public), description = COALESCE($2, description), updated_at = $3
             WHERE id = $4",
        )
        .bind(public.map(i64::from))
        .bind(description)
        .bind(now_ms())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_package(&self, id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM packages WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn add_version(
        &self,
        package_id: &str,
        version: &str,
        digest: &str,
        size: u64,
        meta: &serde_json::Value,
        by: &str,
    ) -> ApiResult<bool> {
        let now = now_ms();
        let added = sqlx::query(
            "INSERT INTO package_versions (package_id, version, digest, size, meta, published_by, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (package_id, version) DO NOTHING",
        )
        .bind(package_id)
        .bind(version)
        .bind(digest)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .bind(meta.to_string())
        .bind(by)
        .bind(now)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1;
        sqlx::query("UPDATE packages SET updated_at = $1 WHERE id = $2")
            .bind(now)
            .bind(package_id)
            .execute(self.pool())
            .await?;
        Ok(added)
    }

    pub async fn versions(&self, package_id: &str) -> ApiResult<Vec<PackageVersion>> {
        let rows = sqlx::query(
            "SELECT version, digest, size, meta, published_by, created_at FROM package_versions
             WHERE package_id = $1 ORDER BY created_at",
        )
        .bind(package_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(version_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn version(
        &self,
        package_id: &str,
        version: &str,
    ) -> ApiResult<Option<PackageVersion>> {
        let row = sqlx::query(
            "SELECT version, digest, size, meta, published_by, created_at FROM package_versions
             WHERE package_id = $1 AND version = $2",
        )
        .bind(package_id)
        .bind(version)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(version_row).transpose()?)
    }

    pub async fn delete_version(&self, package_id: &str, version: &str) -> ApiResult<bool> {
        sqlx::query("DELETE FROM package_tags WHERE package_id = $1 AND version = $2")
            .bind(package_id)
            .bind(version)
            .execute(self.pool())
            .await?;
        Ok(
            sqlx::query("DELETE FROM package_versions WHERE package_id = $1 AND version = $2")
                .bind(package_id)
                .bind(version)
                .execute(self.pool())
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn set_tag(&self, package_id: &str, tag: &str, version: &str) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO package_tags (package_id, tag, version, updated_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (package_id, tag) DO UPDATE SET version = excluded.version, updated_at = excluded.updated_at",
        )
        .bind(package_id)
        .bind(tag)
        .bind(version)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_tag(&self, package_id: &str, tag: &str) -> ApiResult<bool> {
        Ok(
            sqlx::query("DELETE FROM package_tags WHERE package_id = $1 AND tag = $2")
                .bind(package_id)
                .bind(tag)
                .execute(self.pool())
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn tags(&self, package_id: &str) -> ApiResult<Vec<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT tag, version FROM package_tags WHERE package_id = $1 ORDER BY tag",
        )
        .bind(package_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn tag(&self, package_id: &str, tag: &str) -> ApiResult<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT version FROM package_tags WHERE package_id = $1 AND tag = $2",
        )
        .bind(package_id)
        .bind(tag)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn add_blob(&self, package_id: &str, digest: &str, size: u64) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO package_blobs (package_id, digest, size) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(package_id)
        .bind(digest)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Whether an image has this blob: uploaded to it, or a manifest of it.
    pub async fn has_blob(&self, package_id: &str, digest: &str) -> ApiResult<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM package_blobs WHERE package_id = $1 AND digest = $2)
                  + (SELECT COUNT(*) FROM package_versions WHERE package_id = $1 AND digest = $2)",
        )
        .bind(package_id)
        .bind(digest)
        .fetch_one(self.pool())
        .await?;
        Ok(n > 0)
    }

    /// Bytes of an owner's packages (blobs counted once per image).
    pub async fn package_bytes(&self, package_id: &str) -> ApiResult<i64> {
        let n: Option<i64> = sqlx::query_scalar(
            "SELECT CAST((SELECT COALESCE(SUM(size), 0) FROM package_blobs WHERE package_id = $1)
                  + (SELECT COALESCE(SUM(size), 0) FROM package_versions WHERE package_id = $1) AS BIGINT)",
        )
        .bind(package_id)
        .fetch_one(self.pool())
        .await?;
        Ok(n.unwrap_or(0))
    }

    pub async fn start_upload(&self, package_id: &str) -> ApiResult<String> {
        let id = new_id("upl").replace('_', "-");
        sqlx::query("INSERT INTO package_uploads (id, package_id, created_at) VALUES ($1, $2, $3)")
            .bind(&id)
            .bind(package_id)
            .bind(now_ms())
            .execute(self.pool())
            .await?;
        Ok(id)
    }

    pub async fn upload(&self, id: &str) -> ApiResult<Option<(String, i64)>> {
        Ok(
            sqlx::query_as("SELECT package_id, size FROM package_uploads WHERE id = $1")
                .bind(id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn set_upload_size(&self, id: &str, size: u64) -> ApiResult<()> {
        sqlx::query("UPDATE package_uploads SET size = $1 WHERE id = $2")
            .bind(i64::try_from(size).unwrap_or(i64::MAX))
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn end_upload(&self, id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM package_uploads WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Container repositories a caller may list (`_catalog`): public ones,
    /// and those of the given owners.
    pub async fn container_names(&self, owners: &[String]) -> ApiResult<Vec<String>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT name, owner_id, public FROM packages WHERE kind = 'container' ORDER BY name",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .filter(|(_, owner, public)| *public != 0 || owners.contains(owner))
            .map(|(name, ..)| name)
            .collect())
    }
}
