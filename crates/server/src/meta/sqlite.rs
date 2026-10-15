//! Metadata in SQLite (WAL): one writer connection and a pool of read-only
//! connections, all used from blocking threads.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use super::{
    Move, NewReview, RawEvent, RawPresence, RawToken, RepoSummary, Review, Stale, TokenRow,
};
use crate::auth::TokenInfo;
use crate::error::{ApiError, ApiResult};

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
CREATE INDEX IF NOT EXISTS events_by_repo ON events (repo, id);
CREATE TABLE IF NOT EXISTS reviews (
    id INTEGER PRIMARY KEY, repo TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL, author TEXT NOT NULL, state TEXT NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, closed_by TEXT, merged_commit TEXT);
CREATE INDEX IF NOT EXISTS reviews_by_repo ON reviews (repo, state);
";

/// Columns added after the first release, added to older databases.
const MIGRATIONS: &[&str] = &["ALTER TABLE tokens ADD COLUMN expires_at INTEGER"];

const REVIEW_COLUMNS: &str = "id, source, target, title, body, author, state, created_at, updated_at, closed_by, merged_commit";

#[derive(Clone)]
pub struct Sqlite(Arc<Pool>);

struct Pool {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next: AtomicUsize,
}

fn lock(m: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pool {
    fn reader(&self) -> MutexGuard<'_, Connection> {
        // A free reader if there is one, else wait on the next in turn.
        for r in &self.readers {
            if let Ok(guard) = r.try_lock() {
                return guard;
            }
        }
        lock(&self.readers[self.next.fetch_add(1, Ordering::Relaxed) % self.readers.len()])
    }
}

fn review_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Review> {
    Ok(Review {
        id: r.get(0)?,
        source: r.get(1)?,
        target: r.get(2)?,
        title: r.get(3)?,
        body: r.get(4)?,
        author: r.get(5)?,
        state: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        closed_by: r.get(9)?,
        merged_commit: r.get(10)?,
    })
}

impl Sqlite {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let writer = Connection::open(path)?;
        writer.busy_timeout(Duration::from_secs(30))?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        writer.pragma_update(None, "synchronous", "NORMAL")?;
        writer.execute_batch(SCHEMA)?;
        for m in MIGRATIONS {
            // Fails with "duplicate column" once applied.
            let _ = writer.execute_batch(m);
        }
        let readers = (0..std::thread::available_parallelism().map_or(4, std::num::NonZero::get))
            .map(|_| {
                let r = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                r.busy_timeout(Duration::from_secs(30))?;
                Ok(Mutex::new(r))
            })
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self(Arc::new(Pool {
            writer: Mutex::new(writer),
            readers,
            next: AtomicUsize::new(0),
        })))
    }

    /// Runs `f` with the writer on a blocking thread.
    async fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> ApiResult<T> {
        let pool = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || f(&mut lock(&pool.writer)))
            .await
            .map_err(|e| ApiError::internal("internal", e))?
            .map_err(ApiError::from)
    }

    /// Runs `f` with a read-only connection on a blocking thread.
    async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> ApiResult<T> {
        let pool = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || f(&mut pool.reader()))
            .await
            .map_err(|e| ApiError::internal("internal", e))?
            .map_err(ApiError::from)
    }

    pub async fn ping(&self) -> ApiResult<()> {
        self.read(|c| c.query_row("SELECT 1", [], |_| Ok(()))).await
    }

    pub async fn create_repo(&self, name: &str, protect: &[String], at: &str) -> ApiResult<bool> {
        let (name, protect, at) = (name.to_owned(), protect.to_vec(), at.to_owned());
        self.write(move |c| {
            let tx = c.transaction()?;
            let created = tx.execute(
                "INSERT INTO repos (name, created_at) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                params![name, at],
            )? == 1;
            for p in &protect {
                tx.execute(
                    "INSERT INTO protected (repo, pattern) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                    params![name, p],
                )?;
            }
            tx.commit()?;
            Ok(created)
        })
        .await
    }

    pub async fn repo_public(&self, name: &str) -> ApiResult<Option<bool>> {
        let name = name.to_owned();
        self.read(move |c| {
            c.query_row("SELECT public FROM repos WHERE name = ?1", [name], |r| {
                r.get(0)
            })
            .optional()
        })
        .await
    }

    pub async fn public_repos(&self) -> ApiResult<Vec<RepoSummary>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT r.name,
                    (SELECT COUNT(*) FROM bookmarks b WHERE b.repo = r.name),
                    (SELECT MAX(at) FROM events e WHERE e.repo = r.name)
                 FROM repos r WHERE r.public ORDER BY r.name",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(RepoSummary {
                    name: r.get(0)?,
                    bookmarks: r.get(1)?,
                    last_activity: r.get(2)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn bookmarks(&self, repo: &str) -> ApiResult<BTreeMap<String, String>> {
        let repo = repo.to_owned();
        self.read(move |c| {
            let mut stmt = c.prepare("SELECT name, target FROM bookmarks WHERE repo = ?1")?;
            let rows = stmt.query_map([repo], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .await
    }

    pub async fn bookmark(&self, repo: &str, name: &str) -> ApiResult<Option<String>> {
        let (repo, name) = (repo.to_owned(), name.to_owned());
        self.read(move |c| {
            c.query_row(
                "SELECT target FROM bookmarks WHERE repo = ?1 AND name = ?2",
                params![repo, name],
                |r| r.get(0),
            )
            .optional()
        })
        .await
    }

    pub async fn protected(&self, repo: &str) -> ApiResult<Vec<String>> {
        let repo = repo.to_owned();
        self.read(move |c| {
            let mut stmt =
                c.prepare("SELECT pattern FROM protected WHERE repo = ?1 ORDER BY pattern")?;
            let rows = stmt.query_map([repo], |r| r.get(0))?;
            rows.collect()
        })
        .await
    }

    pub async fn cas_bookmark(&self, m: &Move<'_>) -> ApiResult<Result<(), Stale>> {
        let detail = m.detail();
        let (repo, name, actor, at) = (
            m.repo.to_owned(),
            m.name.to_owned(),
            m.actor.to_owned(),
            m.at.to_owned(),
        );
        let (old, new) = (m.old.map(str::to_owned), m.new.map(str::to_owned));
        self.write(move |c| {
            let tx = c.transaction()?;
            let moved = match (&old, &new) {
                (None, Some(new)) => tx.execute(
                    "INSERT INTO bookmarks (repo, name, target, updated_at, updated_by)
                     VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
                    params![repo, name, new, at, actor],
                )?,
                (Some(old), Some(new)) => tx.execute(
                    "UPDATE bookmarks SET target = ?3, updated_at = ?4, updated_by = ?5
                     WHERE repo = ?1 AND name = ?2 AND target = ?6",
                    params![repo, name, new, at, actor, old],
                )?,
                (Some(old), None) => tx.execute(
                    "DELETE FROM bookmarks WHERE repo = ?1 AND name = ?2 AND target = ?3",
                    params![repo, name, old],
                )?,
                (None, None) => usize::from(
                    tx.query_row(
                        "SELECT 1 FROM bookmarks WHERE repo = ?1 AND name = ?2",
                        params![repo, name],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_none(),
                ),
            };
            if moved == 0 {
                let current = tx
                    .query_row(
                        "SELECT target FROM bookmarks WHERE repo = ?1 AND name = ?2",
                        params![repo, name],
                        |r| r.get(0),
                    )
                    .optional()?;
                return Ok(Err(Stale(current)));
            }
            tx.execute(
                "INSERT INTO events (repo, at, actor, kind, detail) VALUES (?1, ?2, ?3, 'bookmark', ?4)",
                params![repo, at, actor, detail],
            )?;
            tx.commit()?;
            Ok(Ok(()))
        })
        .await
    }

    pub async fn add_event(
        &self,
        repo: &str,
        at: &str,
        actor: &str,
        kind: &str,
        detail: &str,
    ) -> ApiResult<()> {
        let v = [repo, at, actor, kind, detail].map(str::to_owned);
        self.write(move |c| {
            c.execute(
                "INSERT INTO events (repo, at, actor, kind, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params_from_iter(v.iter()),
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn events(&self, repo: &str, limit: i64) -> ApiResult<Vec<RawEvent>> {
        let repo = repo.to_owned();
        self.read(move |c| {
            let mut stmt = c.prepare(
                "SELECT at, actor, kind, detail FROM events WHERE repo = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![repo, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .await
    }

    pub async fn insert_token(
        &self,
        hash: &str,
        info: &TokenInfo,
        allow: &str,
        at: &str,
        expires_at: Option<i64>,
    ) -> ApiResult<()> {
        let (hash, info, allow, at) = (
            hash.to_owned(),
            info.clone(),
            allow.to_owned(),
            at.to_owned(),
        );
        self.write(move |c| {
            c.execute(
                "INSERT INTO tokens (hash, name, kind, on_behalf_of, allow, admin, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![hash, info.name, info.kind, info.on_behalf_of, allow, info.admin, at, expires_at],
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn token(&self, hash: &str, now: i64) -> ApiResult<Option<RawToken>> {
        let hash = hash.to_owned();
        self.read(move |c| {
            c.query_row(
                "SELECT name, kind, on_behalf_of, allow, admin FROM tokens
                 WHERE hash = ?1 AND (expires_at IS NULL OR expires_at > ?2)",
                params![hash, now],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
        })
        .await
    }

    pub async fn tokens(&self) -> ApiResult<Vec<TokenRow>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT name, kind, allow, admin, created_at, expires_at FROM tokens ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(TokenRow {
                    name: r.get(0)?,
                    kind: r.get(1)?,
                    allow: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                    admin: r.get(3)?,
                    created_at: r.get(4)?,
                    expires_at: r.get(5)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn revoke_tokens(&self, name: &str) -> ApiResult<u64> {
        let name = name.to_owned();
        self.write(move |c| c.execute("DELETE FROM tokens WHERE name = ?1", [name]))
            .await
            .map(|n| n as u64)
    }

    pub async fn put_presence(
        &self,
        repo: &str,
        client: &str,
        payload: &str,
        at: i64,
    ) -> ApiResult<()> {
        let (repo, client, payload) = (repo.to_owned(), client.to_owned(), payload.to_owned());
        self.write(move |c| {
            c.execute(
                "INSERT INTO presence (repo, client, payload, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (repo, client) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
                params![repo, client, payload, at],
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn presence(&self, repo: &str, cutoff: i64) -> ApiResult<Vec<RawPresence>> {
        let repo = repo.to_owned();
        self.read(move |c| {
            let mut stmt = c.prepare(
                "SELECT client, payload, updated_at FROM presence
                 WHERE repo = ?1 AND updated_at > ?2 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![repo, cutoff], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            rows.collect()
        })
        .await
    }

    pub async fn prune_presence(&self, cutoff: i64) -> ApiResult<u64> {
        self.write(move |c| c.execute("DELETE FROM presence WHERE updated_at <= ?1", [cutoff]))
            .await
            .map(|n| n as u64)
    }

    pub async fn open_review_for(
        &self,
        repo: &str,
        source: &str,
        target: &str,
    ) -> ApiResult<Option<i64>> {
        let (repo, source, target) = (repo.to_owned(), source.to_owned(), target.to_owned());
        self.read(move |c| {
            c.query_row(
                "SELECT id FROM reviews WHERE repo = ?1 AND source = ?2 AND target = ?3 AND state = 'open'",
                params![repo, source, target],
                |r| r.get(0),
            )
            .optional()
        })
        .await
    }

    pub async fn insert_review(&self, r: &NewReview<'_>) -> ApiResult<i64> {
        let v = [r.repo, r.source, r.target, r.title, r.body, r.author, r.at].map(str::to_owned);
        self.write(move |c| {
            c.query_row(
                "INSERT INTO reviews (repo, source, target, title, body, author, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, ?7) RETURNING id",
                rusqlite::params_from_iter(v.iter()),
                |r| r.get(0),
            )
        })
        .await
    }

    pub async fn review(&self, repo: &str, id: i64) -> ApiResult<Option<Review>> {
        let repo = repo.to_owned();
        self.read(move |c| {
            c.query_row(
                &format!("SELECT {REVIEW_COLUMNS} FROM reviews WHERE repo = ?1 AND id = ?2"),
                params![repo, id],
                review_row,
            )
            .optional()
        })
        .await
    }

    pub async fn reviews(&self, repo: &str, state: &str) -> ApiResult<Vec<Review>> {
        let (repo, state) = (repo.to_owned(), state.to_owned());
        self.read(move |c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {REVIEW_COLUMNS} FROM reviews
                 WHERE repo = ?1 AND (?2 = 'all' OR state = ?2) ORDER BY id DESC LIMIT 200"
            ))?;
            let rows = stmt.query_map(params![repo, state], review_row)?;
            rows.collect()
        })
        .await
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
        let (repo, state, by, at) = (
            repo.to_owned(),
            state.to_owned(),
            by.to_owned(),
            at.to_owned(),
        );
        let merged = merged.map(str::to_owned);
        self.write(move |c| {
            c.execute(
                "UPDATE reviews SET state = ?1, closed_by = ?2, merged_commit = ?3, updated_at = ?4
                 WHERE repo = ?5 AND id = ?6 AND state = 'open'",
                params![state, by, merged, at, repo, id],
            )
            .map(|n| n == 1)
        })
        .await
    }
}
