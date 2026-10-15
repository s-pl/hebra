//! Server metadata: repositories, bookmarks, tokens, presence, events and
//! reviews.
//!
//! Two backends with the same behaviour: SQLite ([`sqlite`], one file, for a
//! single server or development) and PostgreSQL ([`postgres`], shared by any
//! number of stateless replicas). Every write that must not race (bookmark
//! moves, finishing a review) is a single conditional statement, so it is
//! safe across replicas.

mod postgres;
mod sqlite;

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::auth::TokenInfo;
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub id: i64,
    pub source: String,
    pub target: String,
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_by: Option<String>,
    pub merged_commit: Option<String>,
}

/// A review to create.
pub struct NewReview<'a> {
    pub repo: &'a str,
    pub source: &'a str,
    pub target: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub author: &'a str,
    pub at: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenRow {
    pub name: String,
    pub kind: String,
    pub allow: Vec<String>,
    pub admin: bool,
    pub created_at: String,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoSummary {
    pub name: String,
    pub bookmarks: i64,
    pub last_activity: Option<String>,
}

/// A bookmark move: from `old` to `new`, either of which may be absent.
pub struct Move<'a> {
    pub repo: &'a str,
    pub name: &'a str,
    pub old: Option<&'a str>,
    pub new: Option<&'a str>,
    pub actor: &'a str,
    pub at: &'a str,
}

impl Move<'_> {
    fn detail(&self) -> String {
        json!({ "name": self.name, "old": self.old, "new": self.new }).to_string()
    }
}

/// A bookmark move that lost a race: what the bookmark points to now.
pub struct Stale(pub Option<String>);

/// A token as stored: its JSON patterns still to be parsed.
type RawToken = (String, String, Option<String>, String, bool);
/// An event as stored: at, actor, kind, detail (JSON).
type RawEvent = (String, String, String, String);
/// Presence as stored: client, payload (JSON), updated at (unix seconds).
type RawPresence = (String, String, i64);

fn token_info((name, kind, on_behalf_of, allow, admin): RawToken) -> ApiResult<TokenInfo> {
    Ok(TokenInfo {
        name,
        kind,
        on_behalf_of,
        allow: serde_json::from_str(&allow)?,
        admin,
    })
}

fn event_json((at, actor, kind, detail): RawEvent) -> ApiResult<Value> {
    Ok(json!({
        "at": at,
        "actor": actor,
        "kind": kind,
        "detail": serde_json::from_str::<Value>(&detail)?,
    }))
}

fn presence_json((client, payload, at): RawPresence) -> ApiResult<Value> {
    let mut v: Value = serde_json::from_str(&payload)?;
    v["client"] = json!(client);
    v["updated_at"] = json!(at);
    Ok(v)
}

#[derive(Clone)]
pub enum Meta {
    Sqlite(sqlite::Sqlite),
    Postgres(postgres::Postgres),
}

/// Calls the same method on whichever backend is in use.
macro_rules! dispatch {
    ($self:ident . $method:ident ( $($arg:expr),* )) => {
        match $self {
            Meta::Sqlite(db) => db.$method($($arg),*).await,
            Meta::Postgres(db) => db.$method($($arg),*).await,
        }
    };
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal("database", e)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal("database", e)
    }
}

impl Meta {
    pub fn sqlite(path: &Path) -> anyhow::Result<Self> {
        Ok(Self::Sqlite(sqlite::Sqlite::open(path)?))
    }

    pub async fn postgres(url: &str, max_connections: u32) -> anyhow::Result<Self> {
        Ok(Self::Postgres(
            postgres::Postgres::connect(url, max_connections).await?,
        ))
    }

    #[must_use]
    pub fn backend(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            Self::Postgres(_) => "postgres",
        }
    }

    /// Fails when the database cannot answer (readiness probe).
    pub async fn ping(&self) -> ApiResult<()> {
        dispatch!(self.ping())
    }

    /// Returns false when the repository already exists.
    pub async fn create_repo(&self, name: &str, protect: &[String], at: &str) -> ApiResult<bool> {
        dispatch!(self.create_repo(name, protect, at))
    }

    /// `None` when the repository does not exist, else whether it is public.
    pub async fn repo_public(&self, name: &str) -> ApiResult<Option<bool>> {
        dispatch!(self.repo_public(name))
    }

    pub async fn public_repos(&self) -> ApiResult<Vec<RepoSummary>> {
        dispatch!(self.public_repos())
    }

    pub async fn bookmarks(&self, repo: &str) -> ApiResult<BTreeMap<String, String>> {
        dispatch!(self.bookmarks(repo))
    }

    pub async fn bookmark(&self, repo: &str, name: &str) -> ApiResult<Option<String>> {
        dispatch!(self.bookmark(repo, name))
    }

    pub async fn protected(&self, repo: &str) -> ApiResult<Vec<String>> {
        dispatch!(self.protected(repo))
    }

    /// Moves a bookmark only if it still points to `old`, and records the
    /// event in the same transaction.
    pub async fn cas_bookmark(&self, m: &Move<'_>) -> ApiResult<Result<(), Stale>> {
        dispatch!(self.cas_bookmark(m))
    }

    pub async fn add_event(
        &self,
        repo: &str,
        at: &str,
        actor: &str,
        kind: &str,
        detail: &Value,
    ) -> ApiResult<()> {
        dispatch!(self.add_event(repo, at, actor, kind, &detail.to_string()))
    }

    /// The newest events of a repository.
    pub async fn events(&self, repo: &str, limit: i64) -> ApiResult<Vec<Value>> {
        let rows = dispatch!(self.events(repo, limit))?;
        rows.into_iter().map(event_json).collect()
    }

    pub async fn insert_token(
        &self,
        hash: &str,
        info: &TokenInfo,
        at: &str,
        expires_at: Option<i64>,
    ) -> ApiResult<()> {
        let allow = serde_json::to_string(&info.allow)?;
        dispatch!(self.insert_token(hash, info, &allow, at, expires_at))
    }

    /// The token with this hash, unless it expired.
    pub async fn token(&self, hash: &str, now: i64) -> ApiResult<Option<TokenInfo>> {
        dispatch!(self.token(hash, now))?
            .map(token_info)
            .transpose()
    }

    pub async fn tokens(&self) -> ApiResult<Vec<TokenRow>> {
        dispatch!(self.tokens())
    }

    pub async fn revoke_tokens(&self, name: &str) -> ApiResult<u64> {
        dispatch!(self.revoke_tokens(name))
    }

    pub async fn put_presence(
        &self,
        repo: &str,
        client: &str,
        payload: &Value,
        at: i64,
    ) -> ApiResult<()> {
        dispatch!(self.put_presence(repo, client, &payload.to_string(), at))
    }

    /// Presence newer than `cutoff` (unix seconds), newest first.
    pub async fn presence(&self, repo: &str, cutoff: i64) -> ApiResult<Vec<Value>> {
        let rows = dispatch!(self.presence(repo, cutoff))?;
        rows.into_iter().map(presence_json).collect()
    }

    /// Deletes presence older than `cutoff`.
    pub async fn prune_presence(&self, cutoff: i64) -> ApiResult<u64> {
        dispatch!(self.prune_presence(cutoff))
    }

    pub async fn open_review_for(
        &self,
        repo: &str,
        source: &str,
        target: &str,
    ) -> ApiResult<Option<i64>> {
        dispatch!(self.open_review_for(repo, source, target))
    }

    pub async fn insert_review(&self, review: &NewReview<'_>) -> ApiResult<i64> {
        dispatch!(self.insert_review(review))
    }

    pub async fn review(&self, repo: &str, id: i64) -> ApiResult<Option<Review>> {
        dispatch!(self.review(repo, id))
    }

    /// Reviews in `state` (`open`, `merged`, `closed` or `all`), newest first.
    pub async fn reviews(&self, repo: &str, state: &str) -> ApiResult<Vec<Review>> {
        dispatch!(self.reviews(repo, state))
    }

    /// Closes or merges an open review. False if it was no longer open.
    pub async fn finish_review(
        &self,
        repo: &str,
        id: i64,
        state: &str,
        by: &str,
        merged: Option<&str>,
        at: &str,
    ) -> ApiResult<bool> {
        dispatch!(self.finish_review(repo, id, state, by, merged, at))
    }
}
