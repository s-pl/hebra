//! Export Hebra history to Git through a `git fast-import` stream.
//!
//! The stream is fully determined by the Hebra commits, so exporting the same
//! history twice yields the same Git commit ids: re-running an export only
//! moves the branch. Provenance travels as Git trailers.

use std::collections::{HashMap, HashSet};
use std::io::Write;

use crate::error::{Error, Result};
use crate::ids::ObjectId;
use crate::model::{ActorKind, Commit};
use crate::repo::Tx;
use crate::store::BLOB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ExportStats {
    pub commits: usize,
    pub blobs: usize,
}

/// Git branch names must not contain these (see `git check-ref-format`).
fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("//")
        && !name.ends_with(".lock")
        && !name
            .chars()
            .any(|c| c.is_ascii_control() || " ~^:?*[\\".contains(c) || c == '\x7f')
}

/// Parents-first order of `tip` and its ancestors, without the root commit.
fn ancestry(tx: &mut Tx<'_>, tip: ObjectId) -> Result<Vec<ObjectId>> {
    let mut order = Vec::new();
    let mut done = HashSet::new();
    let mut stack = vec![(tip, false)];
    while let Some((id, expanded)) = stack.pop() {
        if id == tx.root_commit || done.contains(&id) {
            continue;
        }
        if expanded {
            done.insert(id);
            order.push(id);
            continue;
        }
        stack.push((id, true));
        for parent in tx.commit(&id)?.parents.into_iter().rev() {
            if !done.contains(&parent) {
                stack.push((parent, false));
            }
        }
    }
    Ok(order)
}

fn unix_seconds(timestamp: &str) -> i64 {
    timestamp
        .parse::<jiff::Timestamp>()
        .map_or(0, jiff::Timestamp::as_second)
}

/// `Name <>` with characters git rejects in identities removed.
fn identity(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !"<>\n".contains(*c)).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "unknown".to_owned()
    } else {
        clean.to_owned()
    }
}

fn message(id: &ObjectId, c: &Commit) -> String {
    let mut msg = if c.description.trim().is_empty() {
        "(no description)".to_owned()
    } else {
        c.description.trim_end().to_owned()
    };
    msg.push_str("\n\n");
    msg.push_str(&format!("Hebra-Change: {}\n", c.change_id));
    msg.push_str(&format!("Hebra-Commit: {id}\n"));
    if c.author.kind == ActorKind::Agent {
        msg.push_str(&format!("Hebra-Agent: {}\n", c.author.name));
        if let Some(person) = &c.author.on_behalf_of {
            msg.push_str(&format!("Hebra-On-Behalf-Of: {person}\n"));
        }
    }
    if let Some(task) = &c.provenance.task {
        msg.push_str(&format!("Hebra-Task: {}\n", task.replace('\n', " ")));
    }
    if let Some(session) = &c.provenance.session {
        msg.push_str(&format!("Hebra-Session: {session}\n"));
    }
    if !c.conflicts.is_empty() {
        let paths: Vec<&str> = c.conflicts.iter().map(|x| x.path.as_str()).collect();
        msg.push_str(&format!("Hebra-Conflicts: {}\n", paths.join(", ")));
    }
    msg
}

fn data(out: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    writeln!(out, "data {}", bytes.len())?;
    out.write_all(bytes)?;
    writeln!(out)
}

/// Writes a fast-import stream that makes `refs/heads/<branch>` point at the
/// Git equivalent of `tip`.
pub fn write_fast_import(
    tx: &mut Tx<'_>,
    tip: ObjectId,
    branch: &str,
    out: &mut impl Write,
) -> Result<ExportStats> {
    if !valid_branch(branch) {
        return Err(Error::InvalidArgument(format!(
            "`{branch}` is not a valid Git branch name"
        )));
    }
    if tip == tx.root_commit {
        return Err(Error::InvalidArgument(
            "nothing to export: the revision is the root commit".to_owned(),
        ));
    }
    let io = |e: std::io::Error| Error::Io {
        path: "git fast-import".into(),
        source: e,
    };
    let commits = ancestry(tx, tip)?;
    let mut next_mark = 1u64;
    let mut blob_marks: HashMap<ObjectId, u64> = HashMap::new();
    let mut commit_marks: HashMap<ObjectId, u64> = HashMap::new();

    for id in &commits {
        let commit = tx.commit(id)?;
        let tree = tx.tree(&commit.tree)?;

        for entry in tree.files.values() {
            if blob_marks.contains_key(&entry.blob) {
                continue;
            }
            let bytes = tx.store.read(BLOB, &entry.blob)?;
            writeln!(out, "blob\nmark :{next_mark}").map_err(io)?;
            data(out, &bytes).map_err(io)?;
            blob_marks.insert(entry.blob, next_mark);
            next_mark += 1;
        }

        let when = unix_seconds(&commit.timestamp);
        let who = identity(&commit.author.name);
        let parents: Vec<u64> = commit
            .parents
            .iter()
            .filter_map(|p| commit_marks.get(p).copied())
            .collect();
        if parents.is_empty() {
            // A commit on top of root starts a fresh history, even if the
            // branch exists from an earlier export or an earlier root commit.
            writeln!(out, "reset refs/heads/{branch}").map_err(io)?;
        }
        writeln!(out, "commit refs/heads/{branch}\nmark :{next_mark}").map_err(io)?;
        writeln!(
            out,
            "author {who} <> {when} +0000\ncommitter {who} <> {when} +0000"
        )
        .map_err(io)?;
        data(out, message(id, &commit).as_bytes()).map_err(io)?;
        if let Some((first, rest)) = parents.split_first() {
            writeln!(out, "from :{first}").map_err(io)?;
            for p in rest {
                writeln!(out, "merge :{p}").map_err(io)?;
            }
        }
        writeln!(out, "deleteall").map_err(io)?;
        for (path, entry) in &tree.files {
            let mode = match (entry.symlink, entry.executable) {
                (true, _) => "120000",
                (false, true) => "100755",
                (false, false) => "100644",
            };
            let mark = blob_marks[&entry.blob];
            writeln!(out, "M {mode} :{mark} {}", quote_path(path)).map_err(io)?;
        }
        writeln!(out).map_err(io)?;
        commit_marks.insert(*id, next_mark);
        next_mark += 1;
    }
    writeln!(out, "done").map_err(io)?;
    Ok(ExportStats {
        commits: commits.len(),
        blobs: blob_marks.len(),
    })
}

/// Quotes a path for fast-import when it contains special characters.
fn quote_path(path: &str) -> String {
    if !path.contains(['"', '\\', '\n']) && !path.starts_with(' ') {
        return path.to_owned();
    }
    let escaped = path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_follow_git_rules() {
        for ok in ["main", "feature/x", "hebra/agent-1"] {
            assert!(valid_branch(ok), "{ok}");
        }
        for bad in ["", "-x", "a..b", "a b", "a:b", "x.lock", "a//b", "end/"] {
            assert!(!valid_branch(bad), "{bad}");
        }
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(quote_path("src/main.rs"), "src/main.rs");
        assert_eq!(quote_path("we\"ird"), "\"we\\\"ird\"");
    }
}
