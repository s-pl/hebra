//! Claims: short-lived reservations of paths, so agents and people can see
//! who is about to work on what before any conflict exists.

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::error::Result;
use crate::model::Actor;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Claim {
    /// A file, or a directory prefix that covers everything below it.
    pub path: String,
    pub workspace: String,
    pub actor: Actor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; the claim disappears on its own after this.
    pub expires_at: i64,
}

impl Claim {
    /// Whether this claim covers `file`.
    #[must_use]
    pub fn covers(&self, file: &str) -> bool {
        let claim = self.path.trim_end_matches('/');
        claim.is_empty()
            || file == claim
            || file
                .strip_prefix(claim)
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

#[must_use]
pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

pub(crate) fn upsert(conn: &Connection, claim: &Claim) -> Result<()> {
    conn.execute(
        "INSERT INTO claims (path, workspace, actor, note, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (path, workspace) DO UPDATE SET
           actor = excluded.actor, note = excluded.note, expires_at = excluded.expires_at",
        params![
            claim.path,
            claim.workspace,
            serde_json::to_string(&claim.actor)?,
            claim.note,
            claim.created_at,
            claim.expires_at,
        ],
    )?;
    Ok(())
}

/// Releases the given paths, or every claim of the workspace if `paths` is empty.
pub(crate) fn release(conn: &Connection, workspace: &str, paths: &[String]) -> Result<usize> {
    if paths.is_empty() {
        return Ok(conn.execute("DELETE FROM claims WHERE workspace = ?1", [workspace])?);
    }
    let mut released = 0;
    for path in paths {
        released += conn.execute(
            "DELETE FROM claims WHERE workspace = ?1 AND path = ?2",
            params![workspace, path],
        )?;
    }
    Ok(released)
}

/// Active claims; expired ones are purged first.
pub(crate) fn active(conn: &Connection) -> Result<Vec<Claim>> {
    conn.execute("DELETE FROM claims WHERE expires_at <= ?1", [now()])?;
    let mut stmt = conn.prepare(
        "SELECT path, workspace, actor, note, created_at, expires_at FROM claims ORDER BY path, workspace",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (path, workspace, actor, note, created_at, expires_at) = row?;
        out.push(Claim {
            path,
            workspace,
            actor: serde_json::from_str(&actor)?,
            note,
            created_at,
            expires_at,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(path: &str) -> Claim {
        Claim {
            path: path.to_owned(),
            workspace: "w".to_owned(),
            actor: Actor::human("ana"),
            note: None,
            created_at: 0,
            expires_at: 0,
        }
    }

    #[test]
    fn covers_files_and_directories() {
        assert!(claim("src/auth").covers("src/auth"));
        assert!(claim("src/auth").covers("src/auth/login.rs"));
        assert!(claim("src/auth/").covers("src/auth/login.rs"));
        assert!(!claim("src/auth").covers("src/authz.rs"));
        assert!(!claim("src/auth").covers("src"));
    }
}
