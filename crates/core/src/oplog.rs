//! The operation log: every command that changes the repository is recorded
//! with the full [`View`] it produced, so anything can be undone.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Actor, View};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS operations (
    id        INTEGER PRIMARY KEY,
    parent    INTEGER,
    time      TEXT NOT NULL,
    workspace TEXT NOT NULL,
    actor     TEXT NOT NULL,
    command   TEXT NOT NULL,
    kind      TEXT NOT NULL,
    undoes    INTEGER,
    view      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS claims (
    path       TEXT NOT NULL,
    workspace  TEXT NOT NULL,
    actor      TEXT NOT NULL,
    note       TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (path, workspace)
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

pub(crate) fn insert(conn: &Connection, op: &NewOp<'_>) -> Result<i64> {
    let parent: Option<i64> = conn.query_row("SELECT MAX(id) FROM operations", [], |r| r.get(0))?;
    conn.execute(
        "INSERT INTO operations (parent, time, workspace, actor, command, kind, undoes, view)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            parent,
            jiff::Timestamp::now().to_string(),
            op.workspace,
            serde_json::to_string(op.actor)?,
            op.command,
            op.kind.as_str(),
            op.undoes,
            serde_json::to_string(op.view)?,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Operation, String, String)> {
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
        row.get(8)?,
    ))
}

fn hydrate(
    (mut op, actor, view): (Operation, String, String),
    with_view: bool,
) -> Result<Operation> {
    op.actor = serde_json::from_str(&actor)?;
    if with_view {
        op.view = serde_json::from_str(&view)?;
    }
    Ok(op)
}

const COLUMNS: &str = "id, parent, time, workspace, actor, command, kind, undoes, view";

pub(crate) fn get(conn: &Connection, id: i64) -> Result<Operation> {
    let row = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM operations WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?
        .ok_or_else(|| Error::InvalidArgument(format!("operation {id} not found")))?;
    hydrate(row, true)
}

pub(crate) fn head(conn: &Connection) -> Result<Operation> {
    let id: Option<i64> = conn.query_row("SELECT MAX(id) FROM operations", [], |r| r.get(0))?;
    get(conn, id.ok_or(Error::NotAWorkspace)?)
}

/// Most recent operations first, without their views.
pub(crate) fn list(conn: &Connection, limit: usize) -> Result<Vec<Operation>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM operations ORDER BY id DESC LIMIT ?1"
    ))?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map([limit], from_row)?;
    rows.map(|r| hydrate(r?, false)).collect()
}

/// Time of the last operation recorded by each workspace.
pub(crate) fn last_activity(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT workspace, MAX(time) FROM operations GROUP BY workspace")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
