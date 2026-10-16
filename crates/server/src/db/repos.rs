//! Repositories, who may access them, bookmarks (moved by compare-and-swap),
//! events and presence.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::Row;
use sqlx::any::AnyRow;

use super::{Db, flag, new_id, now_ms, rfc3339};
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize)]
pub struct Repo {
    pub id: String,
    pub owner_id: String,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub public: bool,
    pub open_contributions: bool,
    /// `owner/name` of the repository this one was forked from.
    pub forked_from: Option<String>,
    /// Repositories this one reads objects from (forks), nearest first.
    #[serde(skip)]
    pub lineage: Vec<String>,
    pub created_at: String,
}

macro_rules! repo_select {
    ($tail:literal) => {
        concat!(
            "SELECT r.id, r.owner_id, a.handle AS owner, r.name, r.full_name, r.description, r.public,
                    r.open_contributions, p.full_name AS forked_from, r.lineage, r.created_at
             FROM repos r JOIN accounts a ON a.id = r.owner_id LEFT JOIN repos p ON p.id = r.forked_from ",
            $tail
        )
    };
}

impl Repo {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: r.try_get("id")?,
            owner_id: r.try_get("owner_id")?,
            owner: r.try_get("owner")?,
            name: r.try_get("name")?,
            full_name: r.try_get("full_name")?,
            description: r.try_get("description")?,
            public: flag(r, "public")?,
            open_contributions: flag(r, "open_contributions")?,
            forked_from: r.try_get("forked_from")?,
            lineage: super::json_list(r, "lineage")?,
            created_at: rfc3339(r.try_get("created_at")?),
        })
    }
}

/// What someone may do in a repository, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoRole {
    Read,
    Triage,
    Write,
    Maintain,
    Admin,
}

impl RepoRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Triage => "triage",
            Self::Write => "write",
            Self::Maintain => "maintain",
            Self::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> ApiResult<Self> {
        Ok(match s {
            "read" => Self::Read,
            "triage" => Self::Triage,
            "write" => Self::Write,
            "maintain" => Self::Maintain,
            "admin" => Self::Admin,
            other => return Err(ApiError::bad(format!("unknown repository role `{other}`"))),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoSummary {
    #[serde(flatten)]
    pub repo: Repo,
    pub bookmarks: i64,
    pub last_activity: Option<String>,
}

/// A bookmark move: from `old` to `new`, either of which may be absent.
pub struct Move<'a> {
    pub repo_id: &'a str,
    pub name: &'a str,
    pub old: Option<&'a str>,
    pub new: Option<&'a str>,
    pub actor: &'a str,
}

/// A repository to create.
pub struct NewRepo<'a> {
    pub owner_id: &'a str,
    pub name: &'a str,
    pub public: bool,
    pub description: &'a str,
    /// Bookmarks only people may move.
    pub protect: &'a [String],
    /// The repository this one is a fork of.
    pub forked_from: Option<&'a Repo>,
}

/// A bookmark move that lost a race: what the bookmark points to now.
pub struct Stale(pub Option<String>);

impl Db {
    /// Creates `owner_handle/name`.
    pub async fn create_repo(&self, owner_handle: &str, r: &NewRepo<'_>) -> ApiResult<Repo> {
        let NewRepo {
            owner_id,
            name,
            public,
            description,
            protect,
            forked_from,
        } = *r;
        let full_name = format!("{owner_handle}/{name}");
        let full_name = full_name.as_str();
        let lineage: Vec<String> = forked_from
            .map(|parent| {
                std::iter::once(parent.id.clone())
                    .chain(parent.lineage.iter().cloned())
                    .collect()
            })
            .unwrap_or_default();
        let id = new_id("repo");
        let mut tx = self.pool().begin().await?;
        let created = sqlx::query(
            "INSERT INTO repos (id, owner_id, name, full_name, description, public, forked_from, lineage, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT DO NOTHING",
        )
        .bind(&id)
        .bind(owner_id)
        .bind(name)
        .bind(full_name)
        .bind(description)
        .bind(i64::from(public))
        .bind(forked_from.map(|p| p.id.clone()))
        .bind(serde_json::to_string(&lineage)?)
        .bind(now_ms())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if created == 0 {
            return Err(ApiError::conflict(format!(
                "repository `{full_name}` already exists"
            )));
        }
        for p in protect {
            sqlx::query(
                "INSERT INTO protected (repo_id, pattern) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(&id)
            .bind(p)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.repo(full_name)
            .await?
            .ok_or_else(|| ApiError::internal("database", "repository vanished after insert"))
    }

    /// A repository by `owner/name`.
    pub async fn repo(&self, full_name: &str) -> ApiResult<Option<Repo>> {
        let row = sqlx::query(repo_select!("WHERE r.full_name = $1"))
            .bind(full_name)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(Repo::from_row).transpose()?)
    }

    pub async fn repo_by_id(&self, id: &str) -> ApiResult<Option<Repo>> {
        let row = sqlx::query(repo_select!("WHERE r.id = $1"))
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(Repo::from_row).transpose()?)
    }

    pub async fn update_repo(
        &self,
        id: &str,
        description: &str,
        public: bool,
        open_contributions: bool,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE repos SET description = $1, public = $2, open_contributions = $3 WHERE id = $4",
        )
        .bind(description)
        .bind(i64::from(public))
        .bind(i64::from(open_contributions))
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_repo(&self, id: &str) -> ApiResult<()> {
        sqlx::query("DELETE FROM repos WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Repositories (public and private) with their size and last activity:
    /// those of `owner_id`, or all when `None`. Callers filter by what the
    /// viewer may see.
    pub async fn repo_summaries(&self, owner_id: Option<&str>) -> ApiResult<Vec<RepoSummary>> {
        let rows = match owner_id {
            Some(owner) => {
                sqlx::query(repo_select!(
                    "WHERE r.owner_id = $1 ORDER BY r.name LIMIT 1000"
                ))
                .bind(owner)
                .fetch_all(self.pool())
                .await?
            }
            None => {
                sqlx::query(repo_select!("ORDER BY a.handle, r.name LIMIT 1000"))
                    .fetch_all(self.pool())
                    .await?
            }
        };
        let mut out = Vec::new();
        for row in &rows {
            let repo = Repo::from_row(row)?;
            let (bookmarks, last): (i64, Option<i64>) = sqlx::query_as(
                "SELECT (SELECT COUNT(*) FROM bookmarks WHERE repo_id = $1),
                        (SELECT MAX(at) FROM events WHERE repo_id = $2)",
            )
            .bind(&repo.id)
            .bind(&repo.id)
            .fetch_one(self.pool())
            .await?;
            out.push(RepoSummary {
                repo,
                bookmarks,
                last_activity: last.map(rfc3339),
            });
        }
        Ok(out)
    }

    pub async fn count_private_repos(&self, owner_id: &str) -> ApiResult<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM repos WHERE owner_id = $1 AND public = 0")
                .bind(owner_id)
                .fetch_one(self.pool())
                .await?,
        )
    }

    // ---------------------------------------------------------------- access

    /// The strongest role granted directly to any of `principals` (an
    /// account and its teams).
    pub async fn granted_role(
        &self,
        repo_id: &str,
        principals: &[String],
    ) -> ApiResult<Option<RepoRole>> {
        let roles: Vec<(String, String)> =
            sqlx::query_as("SELECT principal_id, role FROM repo_access WHERE repo_id = $1")
                .bind(repo_id)
                .fetch_all(self.pool())
                .await?;
        let mut best = None;
        for (principal, role) in roles {
            if principals.contains(&principal) {
                best = best.max(Some(RepoRole::parse(&role)?));
            }
        }
        Ok(best)
    }

    /// Ids of repositories where any of `principals` has a grant.
    pub async fn granted_repos(&self, principals: &[String]) -> ApiResult<Vec<String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT repo_id, principal_id FROM repo_access")
                .fetch_all(self.pool())
                .await?;
        Ok(rows
            .into_iter()
            .filter(|(_, p)| principals.contains(p))
            .map(|(r, _)| r)
            .collect())
    }

    pub async fn set_access(
        &self,
        repo_id: &str,
        principal_id: &str,
        role: Option<RepoRole>,
    ) -> ApiResult<()> {
        match role {
            Some(role) => sqlx::query(
                "INSERT INTO repo_access (repo_id, principal_id, role) VALUES ($1, $2, $3)
                 ON CONFLICT (repo_id, principal_id) DO UPDATE SET role = excluded.role",
            )
            .bind(repo_id)
            .bind(principal_id)
            .bind(role.as_str()),
            None => sqlx::query("DELETE FROM repo_access WHERE repo_id = $1 AND principal_id = $2")
                .bind(repo_id)
                .bind(principal_id),
        }
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Every grant on a repository: (principal id, role).
    pub async fn access_list(&self, repo_id: &str) -> ApiResult<Vec<(String, RepoRole)>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT principal_id, role FROM repo_access WHERE repo_id = $1 ORDER BY principal_id",
        )
        .bind(repo_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(p, r)| Ok((p, RepoRole::parse(&r)?)))
            .collect()
    }

    // ------------------------------------------------------------- bookmarks

    pub async fn bookmarks(&self, repo_id: &str) -> ApiResult<BTreeMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT name, target FROM bookmarks WHERE repo_id = $1")
                .bind(repo_id)
                .fetch_all(self.pool())
                .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn bookmark(&self, repo_id: &str, name: &str) -> ApiResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT target FROM bookmarks WHERE repo_id = $1 AND name = $2")
                .bind(repo_id)
                .bind(name)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn protected(&self, repo_id: &str) -> ApiResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT pattern FROM protected WHERE repo_id = $1 ORDER BY pattern")
                .bind(repo_id)
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn set_protected(&self, repo_id: &str, patterns: &[String]) -> ApiResult<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("DELETE FROM protected WHERE repo_id = $1")
            .bind(repo_id)
            .execute(&mut *tx)
            .await?;
        for p in patterns {
            sqlx::query("INSERT INTO protected (repo_id, pattern) VALUES ($1, $2)")
                .bind(repo_id)
                .bind(p)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Moves a bookmark only if it still points to `old`, and records the
    /// event in the same transaction. Safe across replicas: each branch is a
    /// single conditional statement.
    pub async fn cas_bookmark(&self, m: &Move<'_>) -> ApiResult<Result<(), Stale>> {
        let at = now_ms();
        let mut tx = self.pool().begin().await?;
        let moved = match (m.old, m.new) {
            (None, Some(new)) => sqlx::query(
                "INSERT INTO bookmarks (repo_id, name, target, updated_at, updated_by)
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
            )
            .bind(m.repo_id)
            .bind(m.name)
            .bind(new)
            .bind(at)
            .bind(m.actor)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            (Some(old), Some(new)) => sqlx::query(
                "UPDATE bookmarks SET target = $1, updated_at = $2, updated_by = $3
                 WHERE repo_id = $4 AND name = $5 AND target = $6",
            )
            .bind(new)
            .bind(at)
            .bind(m.actor)
            .bind(m.repo_id)
            .bind(m.name)
            .bind(old)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            (Some(old), None) => sqlx::query(
                "DELETE FROM bookmarks WHERE repo_id = $1 AND name = $2 AND target = $3",
            )
            .bind(m.repo_id)
            .bind(m.name)
            .bind(old)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
            (None, None) => u64::from(
                sqlx::query("SELECT 1 FROM bookmarks WHERE repo_id = $1 AND name = $2")
                    .bind(m.repo_id)
                    .bind(m.name)
                    .fetch_optional(&mut *tx)
                    .await?
                    .is_none(),
            ),
        };
        if moved == 0 {
            let current: Option<String> =
                sqlx::query_scalar("SELECT target FROM bookmarks WHERE repo_id = $1 AND name = $2")
                    .bind(m.repo_id)
                    .bind(m.name)
                    .fetch_optional(&mut *tx)
                    .await?;
            return Ok(Err(Stale(current)));
        }
        let entry = json!({ "name": m.name, "old": m.old, "new": m.new, "by": m.actor });
        self.append_entry(&mut tx, m.repo_id, "bookmark", &entry)
            .await?;
        let detail = json!({ "name": m.name, "old": m.old, "new": m.new }).to_string();
        sqlx::query("INSERT INTO events (repo_id, at, actor, kind, detail) VALUES ($1, $2, $3, 'bookmark', $4)")
            .bind(m.repo_id)
            .bind(at)
            .bind(m.actor)
            .bind(detail)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Ok(()))
    }

    // ---------------------------------------------------------------- events

    pub async fn add_event(
        &self,
        repo_id: &str,
        actor: &str,
        kind: &str,
        detail: &Value,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO events (repo_id, at, actor, kind, detail) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(repo_id)
        .bind(now_ms())
        .bind(actor)
        .bind(kind)
        .bind(detail.to_string())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The newest events of a repository.
    pub async fn events(&self, repo_id: &str, limit: i64) -> ApiResult<Vec<Value>> {
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT at, actor, kind, detail FROM events WHERE repo_id = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(repo_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(at, actor, kind, detail)| {
                Ok(json!({
                    "at": rfc3339(at),
                    "actor": actor,
                    "kind": kind,
                    "detail": serde_json::from_str::<Value>(&detail)?,
                }))
            })
            .collect()
    }

    // -------------------------------------------------------------- presence

    pub async fn put_presence(
        &self,
        repo_id: &str,
        client: &str,
        payload: &Value,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO presence (repo_id, client, payload, updated_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (repo_id, client) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
        )
        .bind(repo_id)
        .bind(client)
        .bind(payload.to_string())
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Presence newer than `cutoff_ms`, newest first.
    pub async fn presence(&self, repo_id: &str, cutoff_ms: i64) -> ApiResult<Vec<Value>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT client, payload, updated_at FROM presence
             WHERE repo_id = $1 AND updated_at > $2 ORDER BY updated_at DESC",
        )
        .bind(repo_id)
        .bind(cutoff_ms)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(client, payload, at)| {
                let mut v: Value = serde_json::from_str(&payload)?;
                v["client"] = json!(client);
                v["updated_at"] = json!(at / 1000);
                Ok(v)
            })
            .collect()
    }

    pub async fn prune_presence(&self, cutoff_ms: i64) -> ApiResult<u64> {
        Ok(sqlx::query("DELETE FROM presence WHERE updated_at <= $1")
            .bind(cutoff_ms)
            .execute(self.pool())
            .await?
            .rows_affected())
    }
}
