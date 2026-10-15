//! Hebra server: hosts repositories and serves the HTTP API used by `hb push`,
//! `hb pull`, presence, and the web app.
//!
//! Storage: `<data>/hebra-server.db` (SQLite: repos, bookmarks, tokens,
//! presence, events) and `<data>/repos/<name>/objects` (the same
//! content-addressed store as a local repository).
//!
//! Permissions: every write needs a token. A token may move only the
//! bookmarks matching its patterns, and agent tokens can never move a
//! protected bookmark (by default `main`): a person has to do that.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hebra_core::diff;
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{COMMIT, Store, TREE};
use hebra_core::transport;
use hebra_core::{ChangeId, ObjectId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repos (
    name TEXT PRIMARY KEY, created_at TEXT NOT NULL, public INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS bookmarks (
    repo TEXT NOT NULL, name TEXT NOT NULL, target TEXT NOT NULL,
    updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, PRIMARY KEY (repo, name));
CREATE TABLE IF NOT EXISTS protected (
    repo TEXT NOT NULL, pattern TEXT NOT NULL, PRIMARY KEY (repo, pattern));
CREATE TABLE IF NOT EXISTS tokens (
    hash TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
    on_behalf_of TEXT, allow TEXT NOT NULL, admin INTEGER NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS presence (
    repo TEXT NOT NULL, client TEXT NOT NULL, payload TEXT NOT NULL,
    updated_at INTEGER NOT NULL, PRIMARY KEY (repo, client));
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY, repo TEXT NOT NULL, at TEXT NOT NULL, actor TEXT NOT NULL,
    kind TEXT NOT NULL, detail TEXT NOT NULL);
";

/// Presence entries older than this are hidden.
const PRESENCE_TTL_SECS: i64 = 15 * 60;
/// Largest push accepted in one request.
const MAX_PUSH_BYTES: usize = 1024 * 1024 * 1024;

pub struct Server {
    data: PathBuf,
    db: Mutex<Connection>,
    live: tokio::sync::broadcast::Sender<LiveEvent>,
}

/// Something that just happened in a repository, sent to live subscribers.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    pub repo: String,
    pub kind: &'static str,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    pub name: String,
    pub kind: String,
    pub on_behalf_of: Option<String>,
    pub allow: Vec<String>,
    pub admin: bool,
}

impl TokenInfo {
    fn display(&self) -> String {
        match (&self.kind[..], &self.on_behalf_of) {
            ("agent", Some(p)) => format!("{} (agent for {p})", self.name),
            ("agent", None) => format!("{} (agent)", self.name),
            _ => self.name.clone(),
        }
    }
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// `*` matches any run of characters; everything else is literal.
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text.ends_with(last) || text.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    true
}

#[must_use]
pub fn valid_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

fn valid_bookmark(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains("..")
        && !name.starts_with('/')
        && !name.ends_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}

// ---------------------------------------------------------------- errors

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    fn not_found(what: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", what)
    }
    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
}

impl From<hebra_core::Error> for ApiError {
    fn from(e: hebra_core::Error) -> Self {
        match e {
            hebra_core::Error::InvalidArgument(m) => Self::bad(m),
            hebra_core::Error::CorruptObject(_) => {
                Self::new(StatusCode::BAD_REQUEST, "corrupt_object", e.to_string())
            }
            other => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                other.to_string(),
            ),
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "database", e.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::bad(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": { "code": self.code, "message": self.message } });
        (self.status, axum::Json(body)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ----------------------------------------------------------------- server

impl Server {
    pub fn open(data: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data.join("repos"))?;
        let conn = Connection::open(data.join("hebra-server.db"))?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        let (live, _) = tokio::sync::broadcast::channel(1024);
        Ok(Self {
            data: data.to_path_buf(),
            db: Mutex::new(conn),
            live,
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn store(&self, repo: &str) -> Store {
        Store::new(self.data.join("repos").join(repo).join("objects"))
    }

    pub fn create_repo(&self, name: &str, protect: &[String]) -> anyhow::Result<()> {
        anyhow::ensure!(valid_repo_name(name), "invalid repository name `{name}`");
        std::fs::create_dir_all(self.data.join("repos").join(name).join("objects"))?;
        let db = self.db();
        db.execute(
            "INSERT INTO repos (name, created_at) VALUES (?1, ?2)",
            params![name, now()],
        )
        .map_err(|e| anyhow::anyhow!("cannot create `{name}`: {e}"))?;
        for p in protect {
            db.execute(
                "INSERT OR IGNORE INTO protected (repo, pattern) VALUES (?1, ?2)",
                params![name, p],
            )?;
        }
        Ok(())
    }

    /// Creates a token and returns its secret (shown once; only a hash is stored).
    pub fn create_token(&self, info: &TokenInfo) -> anyhow::Result<String> {
        let mut secret = [0u8; 24];
        rand::fill(&mut secret);
        let token = format!("hbt_{}", hex::encode(secret));
        self.db().execute(
            "INSERT INTO tokens (hash, name, kind, on_behalf_of, allow, admin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                hash_token(&token),
                info.name,
                info.kind,
                info.on_behalf_of,
                serde_json::to_string(&info.allow)?,
                info.admin,
                now()
            ],
        )?;
        Ok(token)
    }

    fn token(&self, headers: &HeaderMap) -> ApiResult<Option<TokenInfo>> {
        let Some(value) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Ok(None);
        };
        let token = value.strip_prefix("Bearer ").unwrap_or(value).trim();
        let row = self
            .db()
            .query_row(
                "SELECT name, kind, on_behalf_of, allow, admin FROM tokens WHERE hash = ?1",
                [hash_token(token)],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, bool>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((name, kind, on_behalf_of, allow, admin)) = row else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "unknown token",
            ));
        };
        Ok(Some(TokenInfo {
            name,
            kind,
            on_behalf_of,
            allow: serde_json::from_str(&allow)?,
            admin,
        }))
    }

    fn require_token(&self, headers: &HeaderMap) -> ApiResult<TokenInfo> {
        self.token(headers)?.ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "a token is required",
            )
        })
    }

    fn require_repo(&self, repo: &str, headers: &HeaderMap) -> ApiResult<()> {
        let public: Option<bool> = self
            .db()
            .query_row("SELECT public FROM repos WHERE name = ?1", [repo], |r| {
                r.get(0)
            })
            .optional()?;
        match public {
            None => Err(ApiError::not_found(format!(
                "repository `{repo}` not found"
            ))),
            Some(true) => Ok(()),
            Some(false) => self.require_token(headers).map(|_| ()),
        }
    }

    fn bookmarks(&self, repo: &str) -> ApiResult<BTreeMap<String, String>> {
        let db = self.db();
        let mut stmt = db.prepare("SELECT name, target FROM bookmarks WHERE repo = ?1")?;
        let rows = stmt.query_map([repo], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn protected(&self, repo: &str) -> ApiResult<Vec<String>> {
        let db = self.db();
        let mut stmt =
            db.prepare("SELECT pattern FROM protected WHERE repo = ?1 ORDER BY pattern")?;
        let rows = stmt.query_map([repo], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tells live subscribers of `repo`; nobody listening is fine.
    fn notify(&self, repo: &str, kind: &'static str, data: Value) {
        let _ = self.live.send(LiveEvent {
            repo: repo.to_owned(),
            kind,
            data,
        });
    }

    fn event(&self, repo: &str, actor: &str, kind: &str, detail: &Value) -> ApiResult<()> {
        self.db().execute(
            "INSERT INTO events (repo, at, actor, kind, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![repo, now(), actor, kind, detail.to_string()],
        )?;
        Ok(())
    }

    fn resolve(&self, repo: &str, rev: &str) -> ApiResult<ObjectId> {
        if let Some(target) = self.bookmarks(repo)?.get(rev) {
            return Ok(ObjectId::parse(target)?);
        }
        let id = ObjectId::parse(rev)
            .map_err(|_| ApiError::not_found(format!("revision `{rev}` not found")))?;
        if self.store(repo).has(&id) {
            Ok(id)
        } else {
            Err(ApiError::not_found(format!("revision `{rev}` not found")))
        }
    }
}

// --------------------------------------------------------------- handlers

type St = State<Arc<Server>>;

/// Runs blocking work (SQLite, files) off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

async fn health() -> impl IntoResponse {
    axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn list_repos(State(s): St) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        let names: Vec<String> = {
            let db = s.db();
            let mut stmt = db.prepare("SELECT name FROM repos WHERE public = 1 ORDER BY name")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut repos = Vec::new();
        for name in names {
            let bookmarks = s.bookmarks(&name)?;
            let last: Option<String> = s
                .db()
                .query_row("SELECT MAX(at) FROM events WHERE repo = ?1", [&name], |r| {
                    r.get(0)
                })
                .optional()?
                .flatten();
            repos
                .push(json!({ "name": name, "bookmarks": bookmarks.len(), "last_activity": last }));
        }
        Ok(axum::Json(json!({ "repos": repos })))
    })
    .await
}

#[derive(Deserialize)]
struct CreateRepo {
    name: String,
    #[serde(default)]
    protect: Option<Vec<String>>,
}

async fn create_repo(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CreateRepo>,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        let token = s.require_token(&headers)?;
        if !token.admin {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "only admin tokens can create repositories",
            ));
        }
        let protect = body.protect.unwrap_or_else(|| vec!["main".to_owned()]);
        s.create_repo(&body.name, &protect)
            .map_err(|e| ApiError::bad(e.to_string()))?;
        Ok(axum::Json(
            json!({ "name": body.name, "protected": protect }),
        ))
    })
    .await
}

async fn repo_info(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        Ok(axum::Json(json!({
            "name": repo,
            "bookmarks": s.bookmarks(&repo)?,
            "protected": s.protected(&repo)?,
        })))
    })
    .await
}

#[derive(Deserialize)]
struct PackRequest {
    want: Vec<String>,
    #[serde(default)]
    have: Vec<String>,
}

async fn pack(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<PackRequest>,
) -> ApiResult<Response> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let store = s.store(&repo);
        let parse = |ids: &[String]| {
            ids.iter()
                .map(|i| ObjectId::parse(i))
                .collect::<Result<Vec<_>, _>>()
        };
        let want = parse(&body.want)?;
        let have: HashSet<ObjectId> = parse(&body.have)?
            .into_iter()
            .filter(|h| store.has(h))
            .collect();
        for w in &want {
            if !store.has(w) {
                return Err(ApiError::not_found(format!("object {w} not found")));
            }
        }
        let ids = transport::closure(&store, &want, &have)?;
        let bytes = transport::encode(&transport::collect(&store, &ids)?);
        Ok(([(header::CONTENT_TYPE, "application/x-hebra-pack")], bytes).into_response())
    })
    .await
}

async fn receive_objects(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        s.require_token(&headers)?;
        let objects = transport::decode(&body)?;
        let new = transport::store_all(&s.store(&repo), &objects)?;
        Ok(axum::Json(json!({ "received": objects.len(), "new": new })))
    })
    .await
}

#[derive(Deserialize)]
struct UpdateBookmark {
    name: String,
    old: Option<String>,
    new: Option<String>,
}

async fn update_bookmark(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpdateBookmark>,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let token = s.require_token(&headers)?;
        if !valid_bookmark(&body.name) {
            return Err(ApiError::bad(format!("invalid bookmark name `{}`", body.name)));
        }
        if !token.allow.iter().any(|p| glob_match(p, &body.name)) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                format!("token `{}` may not move `{}` (allowed: {})", token.name, body.name, token.allow.join(", ")),
            ));
        }
        let protected = s.protected(&repo)?.iter().any(|p| glob_match(p, &body.name));
        if protected && token.kind == "agent" {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "needs_human",
                format!("`{}` is protected: agents push to their own bookmarks and a person moves `{}`", body.name, body.name),
            ));
        }
        if let Some(new) = &body.new {
            let id = ObjectId::parse(new)?;
            if !transport::is_complete(&s.store(&repo), &id)? {
                return Err(ApiError::bad(format!("commit {new} or its objects are missing; send them first")));
            }
        }
        let db = s.db();
        let current: Option<String> = db
            .query_row("SELECT target FROM bookmarks WHERE repo = ?1 AND name = ?2", params![repo, body.name], |r| r.get(0))
            .optional()?;
        if current != body.old {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "stale",
                format!("`{}` moved on the server (now {}); pull and try again", body.name, current.as_deref().unwrap_or("absent")),
            ));
        }
        match &body.new {
            Some(new) => db.execute(
                "INSERT INTO bookmarks (repo, name, target, updated_at, updated_by) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (repo, name) DO UPDATE SET target = excluded.target,
                   updated_at = excluded.updated_at, updated_by = excluded.updated_by",
                params![repo, body.name, new, now(), token.display()],
            )?,
            None => db.execute("DELETE FROM bookmarks WHERE repo = ?1 AND name = ?2", params![repo, body.name])?,
        };
        drop(db);
        s.event(&repo, &token.display(), "bookmark", &json!({ "name": body.name, "old": body.old, "new": body.new }))?;
        s.notify(
            &repo,
            "bookmark",
            json!({ "name": body.name, "old": body.old, "new": body.new, "actor": token.display() }),
        );
        Ok(axum::Json(json!({ "name": body.name, "target": body.new })))
    })
    .await
}

#[derive(Deserialize)]
struct LogQuery {
    rev: Option<String>,
    limit: Option<usize>,
}

fn commit_json(id: &ObjectId, c: &Commit) -> Value {
    json!({
        "commit_id": id.hex(),
        "change_id": c.change_id.encode(),
        "parents": c.parents.iter().map(ObjectId::hex).collect::<Vec<_>>(),
        "description": c.description,
        "author": c.author,
        "timestamp": c.timestamp,
        "provenance": c.provenance,
        "conflicts": c.conflicts,
    })
}

async fn log(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Query<LogQuery>,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let store = s.store(&repo);
        let rev = q.rev.unwrap_or_else(|| "main".to_owned());
        let limit = q.limit.unwrap_or(50).min(500);
        let start = s.resolve(&repo, &rev)?;
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([start]);
        let mut commits: Vec<(ObjectId, Commit)> = Vec::new();
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id) || commits.len() >= limit * 4 {
                continue;
            }
            let c: Commit = store.read_json(COMMIT, &id)?;
            if c.change_id == ChangeId::root() && c.parents.is_empty() {
                continue;
            }
            queue.extend(c.parents.iter().copied());
            commits.push((id, c));
        }
        commits.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
        commits.truncate(limit);
        let bookmarks = s.bookmarks(&repo)?;
        let entries: Vec<Value> = commits
            .iter()
            .map(|(id, c)| {
                let mut v = commit_json(id, c);
                let labels: Vec<&String> = bookmarks
                    .iter()
                    .filter(|(_, t)| **t == id.hex())
                    .map(|(n, _)| n)
                    .collect();
                v["bookmarks"] = json!(labels);
                v
            })
            .collect();
        Ok(axum::Json(json!({ "rev": rev, "commits": entries })))
    })
    .await
}

async fn commit(
    State(s): St,
    UrlPath((repo, id)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let store = s.store(&repo);
        let id = s.resolve(&repo, &id)?;
        let c: Commit = store.read_json(COMMIT, &id)?;
        let tree: Tree = store.read_json(TREE, &c.tree)?;
        let base = match c.parents.first() {
            Some(p) => {
                let parent: Commit = store.read_json(COMMIT, p)?;
                store.read_json(TREE, &parent.tree)?
            }
            None => Tree::default(),
        };
        let files = diff::diff_trees(&store, &base, &tree, true)?;
        let mut v = commit_json(&id, &c);
        v["files"] = json!(files);
        Ok(axum::Json(v))
    })
    .await
}

#[derive(Deserialize)]
struct TreeQuery {
    path: Option<String>,
    /// Return every file path in the tree (for file explorers).
    #[serde(default)]
    recursive: bool,
}

async fn tree(
    State(s): St,
    UrlPath((repo, id)): UrlPath<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<TreeQuery>,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let store = s.store(&repo);
        let id = s.resolve(&repo, &id)?;
        let c: Commit = store.read_json(COMMIT, &id)?;
        let tree: Tree = store.read_json(TREE, &c.tree)?;
        if q.recursive {
            let files: Vec<&String> = tree.files.keys().collect();
            return Ok(axum::Json(json!({ "kind": "list", "files": files })));
        }
        let path = q.path.unwrap_or_default().trim_matches('/').to_owned();
        if let Some(entry) = tree.files.get(&path) {
            let bytes = store.read(hebra_core::store::BLOB, &entry.blob)?;
            let binary = bytes.contains(&0);
            return Ok(axum::Json(json!({
                "kind": "file",
                "path": path,
                "size": bytes.len(),
                "binary": binary,
                "content": if binary { Value::Null } else { Value::String(String::from_utf8_lossy(&bytes).into_owned()) },
            })));
        }
        let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
        let mut entries: BTreeMap<String, &str> = BTreeMap::new();
        for p in tree.files.keys() {
            if let Some(rest) = p.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((dir, _)) => entries.insert(dir.to_owned(), "dir"),
                    None => entries.insert(rest.to_owned(), "file"),
                };
            }
        }
        if entries.is_empty() && !path.is_empty() {
            return Err(ApiError::not_found(format!("path `{path}` not found")));
        }
        let mut list: Vec<Value> = entries.into_iter().map(|(name, kind)| json!({ "name": name, "kind": kind })).collect();
        list.sort_by_key(|e| (e["kind"] != "dir", e["name"].as_str().unwrap_or_default().to_owned()));
        Ok(axum::Json(json!({ "kind": "dir", "path": path, "entries": list })))
    })
    .await
}

async fn get_presence(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let cutoff = jiff::Timestamp::now().as_second() - PRESENCE_TTL_SECS;
        let db = s.db();
        let mut stmt = db.prepare("SELECT client, payload, updated_at FROM presence WHERE repo = ?1 AND updated_at > ?2 ORDER BY updated_at DESC")?;
        let rows = stmt.query_map(params![repo, cutoff], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (client, payload, at) = row?;
            let mut v: Value = serde_json::from_str(&payload)?;
            v["client"] = json!(client);
            v["updated_at"] = json!(at);
            out.push(v);
        }
        Ok(axum::Json(json!({ "presence": out })))
    })
    .await
}

#[derive(Deserialize)]
struct PresenceBody {
    client: String,
    #[serde(flatten)]
    payload: Value,
}

async fn post_presence(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<PresenceBody>,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let token = s.require_token(&headers)?;
        let mut payload = body.payload;
        payload["token"] = json!(token.display());
        s.db().execute(
            "INSERT INTO presence (repo, client, payload, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (repo, client) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            params![repo, body.client, payload.to_string(), jiff::Timestamp::now().as_second()],
        )?;
        s.notify(&repo, "presence", json!({ "client": body.client, "actor": token.display() }));
        Ok(axum::Json(json!({ "ok": true })))
    })
    .await
}

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<usize>,
}

async fn events(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> ApiResult<axum::Json<Value>> {
    blocking(move || {
        s.require_repo(&repo, &headers)?;
        let limit = i64::try_from(q.limit.unwrap_or(30).min(200)).unwrap_or(30);
        let db = s.db();
        let mut stmt = db.prepare("SELECT at, actor, kind, detail FROM events WHERE repo = ?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![repo, limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (at, actor, kind, detail) = row?;
            out.push(json!({ "at": at, "actor": actor, "kind": kind, "detail": serde_json::from_str::<Value>(&detail)? }));
        }
        Ok(axum::Json(json!({ "events": out })))
    })
    .await
}

/// Server-Sent Events: bookmark moves and presence updates for one repository,
/// as they happen. Lagging clients get a `resync` event.
async fn stream(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
) -> ApiResult<
    axum::response::sse::Sse<
        impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, axum::Error>>,
    >,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let check = Arc::clone(&s);
    let check_repo = repo.clone();
    blocking(move || check.require_repo(&check_repo, &headers)).await?;
    let hello = tokio_stream::once(Ok(Event::default().event("ready").data("{}")));
    let updates = BroadcastStream::new(s.live.subscribe()).filter_map(move |item| match item {
        Ok(ev) if ev.repo == repo => Some(Event::default().event(ev.kind).json_data(ev.data)),
        Ok(_) => None,
        Err(_) => Some(Ok(Event::default().event("resync").data("{}"))),
    });
    Ok(Sse::new(hello.chain(updates)).keep_alive(KeepAlive::default()))
}

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/repos", get(list_repos).post(create_repo))
        .route("/api/v1/repos/{repo}", get(repo_info))
        .route("/api/v1/repos/{repo}/pack", post(pack))
        .route(
            "/api/v1/repos/{repo}/objects",
            post(receive_objects).layer(DefaultBodyLimit::max(MAX_PUSH_BYTES)),
        )
        .route("/api/v1/repos/{repo}/bookmarks", post(update_bookmark))
        .route("/api/v1/repos/{repo}/log", get(log))
        .route("/api/v1/repos/{repo}/commits/{id}", get(commit))
        .route("/api/v1/repos/{repo}/tree/{id}", get(tree))
        .route(
            "/api/v1/repos/{repo}/presence",
            get(get_presence).post(post_presence),
        )
        .route("/api/v1/repos/{repo}/events", get(events))
        .route("/api/v1/repos/{repo}/stream", get(stream))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(server)
}

/// Serves until the process is stopped.
pub async fn serve(server: Arc<Server>, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    axum::serve(listener, router(server)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("*", "main"));
        assert!(glob_match("agents/*", "agents/claude/oauth"));
        assert!(!glob_match("agents/*", "main"));
        assert!(glob_match("release-*-final", "release-1.2-final"));
        assert!(!glob_match("main", "mainx"));
    }

    #[test]
    fn names() {
        assert!(valid_repo_name("acme-app"));
        assert!(!valid_repo_name("../etc"));
        assert!(valid_bookmark("agents/claude/oauth"));
        assert!(!valid_bookmark("a/../b"));
    }
}
