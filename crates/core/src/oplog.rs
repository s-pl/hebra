//! The operation log: every command that changes the repository is recorded,
//! so anything can be undone.
//!
//! Each operation stores what it changed in the [`View`] (a [`ViewDelta`]),
//! with a full view every [`CHECKPOINT`] operations, and the latest view lives
//! in the `head` table. A view grows with the number of workspaces, so storing
//! it whole in every operation would make hundreds of agents expensive.
//!
//! Commands read the head without taking any lock, do their work, and only
//! then publish in one short transaction. If another process published
//! meanwhile, the two views are merged (see [`merge_views`]), as in Jujutsu:
//! publishing never fails because someone else got there first.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::ids::ObjectId;
use crate::model::{Actor, Tag, View, WorkspaceInfo};

/// Bumped whenever the database or the repository layout changes; written to
/// SQLite's `user_version`. A repository from a newer `hb` is refused instead
/// of being misread.
pub const SCHEMA_VERSION: i64 = 2;

/// Operations between two full views. Reading any old view applies at most
/// this many deltas.
const CHECKPOINT: i64 = 128;

/// How long a command waits for the database before giving up. Publishing
/// takes milliseconds, so this only runs out on a machine in serious trouble.
const BUSY_LIMIT: Duration = Duration::from_secs(120);

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS claims (
    path       TEXT NOT NULL,
    workspace  TEXT NOT NULL,
    actor      TEXT NOT NULL,
    note       TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (path, workspace)
);
CREATE TABLE IF NOT EXISTS head (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    op   INTEGER NOT NULL,
    view TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS commit_index (
    id         BLOB PRIMARY KEY,
    generation INTEGER NOT NULL,
    time       INTEGER NOT NULL,
    parents    BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS git_map (
    remote TEXT NOT NULL,
    hebra  BLOB NOT NULL,
    git    TEXT NOT NULL,
    PRIMARY KEY (remote, hebra)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS git_map_sha ON git_map (remote, git);
CREATE TABLE IF NOT EXISTS notes (
    target TEXT NOT NULL,
    ref    TEXT NOT NULL,
    body   TEXT NOT NULL,
    PRIMARY KEY (target, ref)
) WITHOUT ROWID;
";

const OPERATIONS: &str = "
CREATE TABLE operations (
    id        INTEGER PRIMARY KEY,
    parent    INTEGER,
    time      TEXT NOT NULL,
    workspace TEXT NOT NULL,
    actor     TEXT NOT NULL,
    command   TEXT NOT NULL,
    kind      TEXT NOT NULL,
    undoes    INTEGER,
    view      TEXT,
    delta     TEXT
);
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Init,
    /// Automatic: the working directory changed since the last command.
    Snapshot,
    Command,
    Undo,
    Restore,
}

impl OpKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Snapshot => "snapshot",
            Self::Command => "command",
            Self::Undo => "undo",
            Self::Restore => "restore",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "init" => Self::Init,
            "snapshot" => Self::Snapshot,
            "undo" => Self::Undo,
            "restore" => Self::Restore,
            _ => Self::Command,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Operation {
    pub id: i64,
    pub parent: Option<i64>,
    pub time: String,
    pub workspace: String,
    pub actor: Actor,
    pub command: String,
    pub kind: OpKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undoes: Option<i64>,
    #[serde(skip)]
    pub view: View,
}

pub(crate) struct NewOp<'a> {
    pub workspace: &'a str,
    pub actor: &'a Actor,
    pub command: &'a str,
    pub kind: OpKind,
    pub undoes: Option<i64>,
    pub view: &'a View,
}

// ------------------------------------------------------------ deltas

/// What changed between two views. `None` as a map value means "removed".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDelta {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workspaces: BTreeMap<String, Option<WorkspaceInfo>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    bookmarks: BTreeMap<String, Option<ObjectId>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tags: BTreeMap<String, Option<Tag>>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    heads_added: BTreeSet<ObjectId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    heads_removed: BTreeSet<ObjectId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    remotes: BTreeMap<String, BTreeMap<String, Option<ObjectId>>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    conflicts: BTreeMap<String, Option<BTreeSet<ObjectId>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    stashes_added: Vec<ObjectId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    stashes_removed: Vec<ObjectId>,
}

fn map_delta<V: Clone + PartialEq>(
    from: &BTreeMap<String, V>,
    to: &BTreeMap<String, V>,
) -> BTreeMap<String, Option<V>> {
    let mut out = BTreeMap::new();
    for (key, value) in to {
        if from.get(key) != Some(value) {
            out.insert(key.clone(), Some(value.clone()));
        }
    }
    for key in from.keys() {
        if !to.contains_key(key) {
            out.insert(key.clone(), None);
        }
    }
    out
}

fn apply_map<V: Clone>(map: &mut BTreeMap<String, V>, delta: &BTreeMap<String, Option<V>>) {
    for (key, value) in delta {
        match value {
            Some(value) => {
                map.insert(key.clone(), value.clone());
            }
            None => {
                map.remove(key);
            }
        }
    }
}

impl ViewDelta {
    #[must_use]
    pub fn between(from: &View, to: &View) -> Self {
        let mut remotes = BTreeMap::new();
        let names: BTreeSet<&String> = from.remotes.keys().chain(to.remotes.keys()).collect();
        let empty = BTreeMap::new();
        for name in names {
            let delta = map_delta(
                from.remotes.get(name).unwrap_or(&empty),
                to.remotes.get(name).unwrap_or(&empty),
            );
            if !delta.is_empty() {
                remotes.insert(name.clone(), delta);
            }
        }
        Self {
            workspaces: map_delta(&from.workspaces, &to.workspaces),
            bookmarks: map_delta(&from.bookmarks, &to.bookmarks),
            tags: map_delta(&from.tags, &to.tags),
            heads_added: to.heads.difference(&from.heads).copied().collect(),
            heads_removed: from.heads.difference(&to.heads).copied().collect(),
            remotes,
            conflicts: map_delta(&from.bookmark_conflicts, &to.bookmark_conflicts),
            stashes_added: to
                .stashes
                .iter()
                .filter(|s| !from.stashes.contains(s))
                .copied()
                .collect(),
            stashes_removed: from
                .stashes
                .iter()
                .filter(|s| !to.stashes.contains(s))
                .copied()
                .collect(),
        }
    }

    pub fn apply(&self, view: &mut View) {
        apply_map(&mut view.workspaces, &self.workspaces);
        apply_map(&mut view.bookmarks, &self.bookmarks);
        apply_map(&mut view.tags, &self.tags);
        apply_map(&mut view.bookmark_conflicts, &self.conflicts);
        for id in &self.heads_removed {
            view.heads.remove(id);
        }
        view.heads.extend(self.heads_added.iter().copied());
        for (name, delta) in &self.remotes {
            let remote = view.remotes.entry(name.clone()).or_default();
            apply_map(remote, delta);
            if remote.is_empty() {
                view.remotes.remove(name);
            }
        }
        view.stashes.retain(|s| !self.stashes_removed.contains(s));
        // Newest first; the delta keeps the order they had.
        for id in self.stashes_added.iter().rev() {
            if !view.stashes.contains(id) {
                view.stashes.insert(0, *id);
            }
        }
    }
}

/// Combines what two concurrent operations did to `base`: the result is
/// `theirs` (already published) plus everything we changed.
///
/// Commits are never lost. When both moved the same bookmark, ours wins and
/// theirs is recorded in `bookmark_conflicts` until someone sets the bookmark
/// again; when both moved the same workspace, ours wins and theirs stays
/// visible as a head.
#[must_use]
pub fn merge_views(base: &View, ours: &View, theirs: &View) -> View {
    let delta = ViewDelta::between(base, ours);
    let mut out = theirs.clone();
    for (name, wanted) in &delta.bookmarks {
        let (before, now) = (base.bookmarks.get(name), theirs.bookmarks.get(name));
        if now == before || now == wanted.as_ref() {
            continue;
        }
        if let Some(other) = now {
            out.bookmark_conflicts
                .entry(name.clone())
                .or_default()
                .insert(*other);
        }
    }
    for (name, wanted) in &delta.workspaces {
        let (before, now) = (base.workspaces.get(name), theirs.workspaces.get(name));
        if now != before
            && now != wanted.as_ref()
            && let Some(other) = now
        {
            out.heads.insert(other.wc);
        }
    }
    delta.apply(&mut out);
    // A conflict on a bookmark that ended up where the other side wanted it
    // is no conflict.
    let bookmarks = out.bookmarks.clone();
    out.bookmark_conflicts.retain(|name, others| {
        if let Some(target) = bookmarks.get(name) {
            others.remove(target);
        }
        !others.is_empty()
    });
    out
}

// ------------------------------------------------------------ opening

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// Runs `f` again while SQLite reports the database busy. SQLite's own
/// waiting covers most cases, but some are reported at once, like the moment
/// a process is recovering the log after a crash.
pub(crate) fn retry_busy<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let started = Instant::now();
    let mut delay = Duration::from_millis(1);
    loop {
        match f() {
            Err(Error::Db(e)) if is_busy(&e) && started.elapsed() < BUSY_LIMIT => {
                // Random, so waiting processes do not retry in lockstep.
                let jitter = rand::random_range(0.5..1.5);
                std::thread::sleep(delay.mul_f64(jitter));
                delay = (delay * 2).min(Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// Opens the repository database, creating or upgrading it if needed.
pub(crate) fn open(path: &Path) -> Result<Connection> {
    retry_busy(|| {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(30))?;
        // Every commit reaches the disk: an operation that was reported as
        // done survives a power cut.
        conn.pragma_update(None, "synchronous", "FULL")?;
        // The last process to close would take an exclusive lock to clean up
        // the log; with short-lived processes that is the common case, and
        // whoever opens the database at that moment fails. The log is
        // checkpointed as it grows instead.
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::RepositoryTooNew {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        if version < SCHEMA_VERSION {
            upgrade(&mut conn)?;
        }
        Ok(conn)
    })
}

/// Creates the tables, or converts a repository from an older `hb`.
fn upgrade(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another process may have done it while we waited for the lock.
    let version: i64 = txn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    txn.execute_batch(TABLES)?;
    let exists: Option<String> = txn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'operations'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_some() {
        convert_full_views(&txn)?;
    } else {
        txn.execute_batch(OPERATIONS)?;
    }
    txn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    txn.commit()?;
    Ok(())
}

/// Version 1 kept the whole view in every operation.
fn convert_full_views(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE operations RENAME TO operations_v1;")?;
    conn.execute_batch(OPERATIONS)?;
    let mut previous: Option<View> = None;
    let mut last = None;
    {
        let mut rows = conn.prepare(
            "SELECT id, parent, time, workspace, actor, command, kind, undoes, view
             FROM operations_v1 ORDER BY id",
        )?;
        let mut insert = conn.prepare(
            "INSERT INTO operations (id, parent, time, workspace, actor, command, kind, undoes, view, delta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        let mut cursor = rows.query([])?;
        while let Some(row) = cursor.next()? {
            let id: i64 = row.get(0)?;
            let text: String = row.get(8)?;
            let view: View = serde_json::from_str(&text)?;
            let delta = match &previous {
                Some(before) if id % CHECKPOINT != 0 => {
                    Some(serde_json::to_string(&ViewDelta::between(before, &view))?)
                }
                _ => None,
            };
            let full = delta.is_none().then_some(&text);
            insert.execute(params![
                id,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                full,
                delta,
            ])?;
            previous = Some(view);
            last = Some((id, text));
        }
    }
    if let Some((id, view)) = last {
        conn.execute(
            "INSERT INTO head (id, op, view) VALUES (1, ?1, ?2)",
            params![id, view],
        )?;
    }
    conn.execute_batch("DROP TABLE operations_v1;")?;
    Ok(())
}

// ------------------------------------------------------------ operations

/// Records an operation. `previous` is the view it was built on, if any.
pub(crate) fn insert(conn: &Connection, op: &NewOp<'_>, previous: Option<&View>) -> Result<i64> {
    let parent = head_id(conn)?;
    let id = parent.unwrap_or(0) + 1;
    let text = serde_json::to_string(op.view)?;
    let delta = match previous {
        Some(before) if id % CHECKPOINT != 0 => {
            Some(serde_json::to_string(&ViewDelta::between(before, op.view))?)
        }
        _ => None,
    };
    let full = delta.is_none().then_some(&text);
    conn.execute(
        "INSERT INTO operations (id, parent, time, workspace, actor, command, kind, undoes, view, delta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            parent,
            jiff::Timestamp::now().to_string(),
            op.workspace,
            serde_json::to_string(op.actor)?,
            op.command,
            op.kind.as_str(),
            op.undoes,
            full,
            delta,
        ],
    )?;
    conn.execute(
        "INSERT INTO head (id, op, view) VALUES (1, ?1, ?2)
         ON CONFLICT (id) DO UPDATE SET op = excluded.op, view = excluded.view",
        params![id, text],
    )?;
    Ok(id)
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Operation, String)> {
    Ok((
        Operation {
            id: row.get(0)?,
            parent: row.get(1)?,
            time: row.get(2)?,
            workspace: row.get(3)?,
            actor: crate::model::Actor::human(""),
            command: row.get(5)?,
            kind: OpKind::parse(&row.get::<_, String>(6)?),
            undoes: row.get(7)?,
            view: View::default(),
        },
        row.get(4)?,
    ))
}

fn hydrate((mut op, actor): (Operation, String)) -> Result<Operation> {
    op.actor = serde_json::from_str(&actor)?;
    Ok(op)
}

const COLUMNS: &str = "id, parent, time, workspace, actor, command, kind, undoes";

/// The view after an operation: the nearest full view, plus deltas.
fn view_at(conn: &Connection, id: i64) -> Result<View> {
    let (start, text): (i64, String) = conn
        .query_row(
            "SELECT id, view FROM operations
             WHERE id <= ?1 AND view IS NOT NULL ORDER BY id DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| Error::InvalidArgument(format!("operation {id} not found")))?;
    let mut view: View = serde_json::from_str(&text)?;
    let mut stmt =
        conn.prepare("SELECT delta FROM operations WHERE id > ?1 AND id <= ?2 ORDER BY id")?;
    let rows = stmt.query_map([start, id], |r| r.get::<_, Option<String>>(0))?;
    for row in rows {
        if let Some(delta) = row? {
            serde_json::from_str::<ViewDelta>(&delta)?.apply(&mut view);
        }
    }
    Ok(view)
}

fn row(conn: &Connection, id: i64) -> Result<Operation> {
    let row = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM operations WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?
        .ok_or_else(|| Error::InvalidArgument(format!("operation {id} not found")))?;
    hydrate(row)
}

pub(crate) fn get(conn: &Connection, id: i64) -> Result<Operation> {
    let mut op = row(conn, id)?;
    op.view = view_at(conn, id)?;
    Ok(op)
}

/// The id of the latest operation.
pub(crate) fn head_id(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn
        .prepare_cached("SELECT op FROM head WHERE id = 1")?
        .query_row([], |r| r.get(0))
        .optional()?)
}

/// The latest operation with its view. Reads never wait for writers.
pub(crate) fn head(conn: &Connection) -> Result<Operation> {
    let (id, text): (i64, String) = conn
        .prepare_cached("SELECT op, view FROM head WHERE id = 1")?
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .ok_or(Error::NotAWorkspace)?;
    let mut op = row(conn, id)?;
    op.view = serde_json::from_str(&text)?;
    Ok(op)
}

/// Most recent operations first, without their views.
pub(crate) fn list(conn: &Connection, limit: usize) -> Result<Vec<Operation>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM operations ORDER BY id DESC LIMIT ?1"
    ))?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map([limit], from_row)?;
    rows.map(|r| hydrate(r?)).collect()
}

/// Time of the last operation recorded by each workspace.
pub(crate) fn last_activity(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT workspace, MAX(time) FROM operations GROUP BY workspace")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every commit any recorded operation points at directly.
pub(crate) fn all_tips(conn: &Connection) -> Result<HashSet<ObjectId>> {
    let mut tips = HashSet::new();
    let mut view = View::default();
    let mut stmt = conn.prepare("SELECT view, delta FROM operations ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        match row? {
            (Some(full), _) => view = serde_json::from_str(&full)?,
            (None, Some(delta)) => serde_json::from_str::<ViewDelta>(&delta)?.apply(&mut view),
            (None, None) => {}
        }
        tips.extend(view.tips());
    }
    Ok(tips)
}

/// Forgets operations older than `before`, except the newest few, so their
/// commits can be collected. Returns how many went.
pub(crate) fn expire(conn: &mut Connection, before: &str, keep: i64) -> Result<usize> {
    let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(head) = head_id(&txn)? else {
        return Ok(0);
    };
    let last: Option<i64> = txn.query_row(
        "SELECT MAX(id) FROM operations WHERE time < ?1 AND id <= ?2",
        params![before, head - keep],
        |r| r.get(0),
    )?;
    let Some(last) = last else {
        return Ok(0);
    };
    let first_kept = last + 1;
    let view = view_at(&txn, first_kept)?;
    txn.execute(
        "UPDATE operations SET view = ?1, delta = NULL, parent = NULL WHERE id = ?2",
        params![serde_json::to_string(&view)?, first_kept],
    )?;
    let removed = txn.execute("DELETE FROM operations WHERE id <= ?1", [last])?;
    txn.commit()?;
    Ok(removed)
}

/// Moves the write-ahead log into the database and empties it.
pub(crate) fn checkpoint(conn: &Connection) -> Result<()> {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActorKind, WorkspaceInfo};

    fn id(n: u8) -> ObjectId {
        ObjectId::from_bytes([n; 32])
    }

    fn ws(wc: u8) -> WorkspaceInfo {
        WorkspaceInfo {
            wc: id(wc),
            path: "/w".to_owned(),
            actor: None,
            task: None,
            created_at: "now".to_owned(),
            bookmark: None,
        }
    }

    fn base() -> View {
        let mut view = View::default();
        view.workspaces.insert("a".to_owned(), ws(1));
        view.workspaces.insert("b".to_owned(), ws(2));
        view.bookmarks.insert("main".to_owned(), id(3));
        view.heads.insert(id(4));
        view
    }

    #[test]
    fn deltas_round_trip() {
        let before = base();
        let mut after = before.clone();
        after.workspaces.insert("a".to_owned(), ws(9));
        after.workspaces.remove("b");
        after.bookmarks.insert("dev".to_owned(), id(5));
        after.bookmarks.remove("main");
        after.heads.remove(&id(4));
        after.heads.insert(id(6));
        after
            .remotes
            .entry("origin".to_owned())
            .or_default()
            .insert("main".to_owned(), id(3));
        after.stashes = vec![id(7), id(8)];
        let delta = ViewDelta::between(&before, &after);
        let json = serde_json::to_string(&delta).unwrap();
        let mut rebuilt = before.clone();
        serde_json::from_str::<ViewDelta>(&json)
            .unwrap()
            .apply(&mut rebuilt);
        assert_eq!(rebuilt, after);
        // And back.
        let mut back = after.clone();
        ViewDelta::between(&after, &before).apply(&mut back);
        assert_eq!(back, before);
    }

    #[test]
    fn independent_work_merges_without_conflict() {
        let start = base();
        let mut ours = start.clone();
        ours.workspaces.insert("a".to_owned(), ws(10));
        ours.bookmarks.insert("ours".to_owned(), id(11));
        let mut theirs = start.clone();
        theirs.workspaces.insert("b".to_owned(), ws(20));
        theirs.bookmarks.insert("theirs".to_owned(), id(21));
        let merged = merge_views(&start, &ours, &theirs);
        assert_eq!(merged.workspaces["a"].wc, id(10));
        assert_eq!(merged.workspaces["b"].wc, id(20));
        assert_eq!(merged.bookmarks.len(), 3);
        assert!(merged.bookmark_conflicts.is_empty());
    }

    #[test]
    fn nothing_is_lost_when_both_move_the_same_thing() {
        let start = base();
        let mut ours = start.clone();
        ours.bookmarks.insert("main".to_owned(), id(10));
        ours.workspaces.insert("a".to_owned(), ws(12));
        let mut theirs = start.clone();
        theirs.bookmarks.insert("main".to_owned(), id(11));
        theirs.workspaces.insert("a".to_owned(), ws(13));
        let merged = merge_views(&start, &ours, &theirs);
        assert_eq!(merged.bookmarks["main"], id(10));
        assert!(merged.bookmark_conflicts["main"].contains(&id(11)));
        assert_eq!(merged.workspaces["a"].wc, id(12));
        assert!(merged.heads.contains(&id(13)));

        // Both moving it to the same place is not a conflict.
        let mut same = start.clone();
        same.bookmarks.insert("main".to_owned(), id(10));
        let merged = merge_views(&start, &ours, &same);
        assert!(merged.bookmark_conflicts.is_empty());
    }

    fn database() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("hebra.db")).unwrap();
        (dir, conn)
    }

    fn record(conn: &Connection, view: &View, previous: Option<&View>) -> i64 {
        insert(
            conn,
            &NewOp {
                workspace: "a",
                actor: &Actor {
                    name: "ana".to_owned(),
                    kind: ActorKind::Human,
                    on_behalf_of: None,
                    email: None,
                },
                command: "test",
                kind: OpKind::Command,
                undoes: None,
                view,
            },
            previous,
        )
        .unwrap()
    }

    #[test]
    fn old_views_are_rebuilt_from_checkpoints_and_deltas() {
        let (_dir, conn) = database();
        let mut views = vec![base()];
        record(&conn, &views[0], None);
        for i in 0..300u32 {
            let mut next = views.last().unwrap().clone();
            next.bookmarks
                .insert(format!("b{}", i % 7), id((i % 200) as u8));
            record(&conn, &next, views.last());
            views.push(next);
        }
        for op in [1, 2, 100, 128, 129, 256, 300, 301] {
            assert_eq!(get(&conn, op).unwrap().view, views[op as usize - 1], "{op}");
        }
        assert_eq!(head(&conn).unwrap().view, *views.last().unwrap());
        // Far smaller than one full view per operation.
        let full: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE view IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(full <= 4, "{full} full views");
    }

    #[test]
    fn expired_operations_leave_later_views_intact() {
        let (_dir, mut conn) = database();
        let mut view = base();
        record(&conn, &view, None);
        for i in 0..200u8 {
            let next = {
                let mut v = view.clone();
                v.bookmarks.insert("main".to_owned(), id(i));
                v
            };
            record(&conn, &next, Some(&view));
            view = next;
        }
        let kept = get(&conn, 150).unwrap().view;
        // Everything is older than the far future, but the newest stay.
        let removed = expire(&mut conn, "9999", 60).unwrap();
        assert_eq!(removed, 141);
        assert_eq!(get(&conn, 150).unwrap().view, kept);
        assert!(get(&conn, 100).is_err());
        assert_eq!(head(&conn).unwrap().view, view);
    }

    #[test]
    fn a_newer_repository_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hebra.db");
        let conn = open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(conn);
        assert!(matches!(open(&path), Err(Error::RepositoryTooNew { .. })));
    }

    #[test]
    fn version_one_databases_are_converted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hebra.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('schema_version', '1'), ('root_commit', 'x');
             CREATE TABLE operations (id INTEGER PRIMARY KEY, parent INTEGER, time TEXT NOT NULL,
               workspace TEXT NOT NULL, actor TEXT NOT NULL, command TEXT NOT NULL,
               kind TEXT NOT NULL, undoes INTEGER, view TEXT NOT NULL);",
        )
        .unwrap();
        let actor = serde_json::to_string(&Actor::human("ana")).unwrap();
        let mut views = Vec::new();
        for i in 1..=5u8 {
            let mut view = base();
            view.bookmarks.insert("main".to_owned(), id(i));
            conn.execute(
                "INSERT INTO operations VALUES (?1, ?2, 't', 'a', ?3, 'x', 'command', NULL, ?4)",
                params![
                    i64::from(i),
                    (i > 1).then(|| i64::from(i) - 1),
                    actor,
                    serde_json::to_string(&view).unwrap()
                ],
            )
            .unwrap();
            views.push(view);
        }
        drop(conn);

        let conn = open(&path).unwrap();
        assert_eq!(head(&conn).unwrap().id, 5);
        assert_eq!(head(&conn).unwrap().view, views[4]);
        assert_eq!(get(&conn, 2).unwrap().view, views[1]);
        assert_eq!(list(&conn, 10).unwrap().len(), 5);
    }
}
