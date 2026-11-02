//! Export Hebra history to Git through a `git fast-import` stream.
//!
//! The stream is fully determined by the Hebra commits, so exporting the same
//! history twice yields the same Git commit ids: re-running an export only
//! moves the branch. Provenance travels as Git trailers.

use std::collections::{HashMap, HashSet};
use std::io::Write;

use crate::error::{Error, Result};
use crate::ids::ObjectId;
use crate::model::{Actor, ActorKind, Commit, Tree};
use crate::repo::Tx;
use crate::store::BLOB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ExportStats {
    pub commits: usize,
    pub blobs: usize,
}

/// Git branch names must not contain these (see `git check-ref-format`).
#[must_use]
pub fn valid_branch(name: &str) -> bool {
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

/// Parents-first order of what `tips` need, stopping at the root commit and
/// at commits Git already has.
fn ancestry(
    tx: &mut Tx<'_>,
    tips: &[ObjectId],
    known: &dyn Fn(&ObjectId) -> Option<String>,
) -> Result<Vec<ObjectId>> {
    let mut order = Vec::new();
    let mut done = HashSet::new();
    let mut stack: Vec<(ObjectId, bool)> = tips.iter().rev().map(|t| (*t, false)).collect();
    while let Some((id, expanded)) = stack.pop() {
        if id == tx.root_commit || done.contains(&id) {
            continue;
        }
        if expanded {
            done.insert(id);
            order.push(id);
            continue;
        }
        if known(&id).is_some() {
            done.insert(id);
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

/// `Name <email>` with characters git rejects in identities removed.
fn identity(actor: &Actor) -> String {
    let clean = |text: &str| -> String { text.chars().filter(|c| !"<>\n".contains(*c)).collect() };
    let name = clean(&actor.name);
    let name = name.trim();
    let name = if name.is_empty() { "unknown" } else { name };
    let email = clean(actor.email.as_deref().unwrap_or(""));
    format!("{name} <{}>", email.trim())
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
    let refs = [(format!("refs/heads/{branch}"), tip)];
    Ok(write_refs(tx, &refs, &|_| None, out)?.stats)
}

/// What [`write_refs`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub stats: ExportStats,
    /// The fast-import mark of each commit, to ask Git for its id.
    pub marks: Vec<(u64, ObjectId)>,
}

/// Writes a fast-import stream that makes each reference point at the Git
/// equivalent of its commit. Commits `known` has an id for are already in
/// Git: new commits are simply built on top of them, so pushing never
/// rewrites history that came from there.
pub fn write_refs(
    tx: &mut Tx<'_>,
    refs: &[(String, ObjectId)],
    known: &dyn Fn(&ObjectId) -> Option<String>,
    out: &mut impl Write,
) -> Result<Exported> {
    let io = |e: std::io::Error| Error::Io {
        path: "git fast-import".into(),
        source: e,
    };
    let tips: Vec<ObjectId> = refs.iter().map(|(_, id)| *id).collect();
    let commits = ancestry(tx, &tips, known)?;
    let branch = refs
        .first()
        .map_or("refs/heads/hebra-export", |(name, _)| name.as_str());
    let mut next_mark = 1u64;
    let mut blob_marks: HashMap<ObjectId, u64> = HashMap::new();
    let mut commit_marks: HashMap<ObjectId, u64> = HashMap::new();
    let empty = tx.store.write_tree(&Tree::default())?;

    for id in &commits {
        let commit = tx.commit(id)?;
        // Only what changed since the first parent (`from`); unchanged
        // directories are not even read.
        let first_parent = commit
            .parents
            .iter()
            .find(|p| commit_marks.contains_key(p) || known(p).is_some())
            .copied();
        let base = match first_parent {
            Some(parent) => tx.commit(&parent)?.tree,
            None => empty,
        };
        let changes = tx.store.tree_changes(&base, &commit.tree)?;

        for entry in changes.iter().filter_map(|(_, e)| e.as_ref()) {
            if entry.gitlink || blob_marks.contains_key(&entry.blob) {
                continue;
            }
            let bytes = tx.store.read(BLOB, &entry.blob)?;
            writeln!(out, "blob\nmark :{next_mark}").map_err(io)?;
            data(out, &bytes).map_err(io)?;
            blob_marks.insert(entry.blob, next_mark);
            next_mark += 1;
        }

        let committer = commit.committer.as_ref().unwrap_or(&commit.author);
        let written = unix_seconds(&commit.timestamp);
        let authored = commit.authored_at.as_deref().map_or(written, unix_seconds);
        let parents: Vec<String> = commit
            .parents
            .iter()
            .filter_map(|p| {
                commit_marks
                    .get(p)
                    .map(|m| format!(":{m}"))
                    .or_else(|| known(p))
            })
            .collect();
        if parents.is_empty() {
            // A commit on top of root starts a fresh history, even if the
            // branch exists from an earlier export or an earlier root commit.
            writeln!(out, "reset {branch}").map_err(io)?;
        }
        writeln!(out, "commit {branch}\nmark :{next_mark}").map_err(io)?;
        writeln!(
            out,
            "author {} {authored} +0000\ncommitter {} {written} +0000",
            identity(&commit.author),
            identity(committer),
        )
        .map_err(io)?;
        data(out, message(id, &commit).as_bytes()).map_err(io)?;
        if let Some((first, rest)) = parents.split_first() {
            writeln!(out, "from {first}").map_err(io)?;
            for p in rest {
                writeln!(out, "merge {p}").map_err(io)?;
            }
        }
        if first_parent.is_none() {
            writeln!(out, "deleteall").map_err(io)?;
        }
        // Deletions first, so a file can become a folder and back.
        for (path, _) in changes.iter().filter(|(_, e)| e.is_none()) {
            writeln!(out, "D {}", quote_path(path)).map_err(io)?;
        }
        for (path, entry) in changes.iter().filter_map(|(p, e)| e.map(|e| (p, e))) {
            if entry.gitlink {
                // Written into the stream as is, so it must be exactly a
                // commit id: anything else could inject fast-import commands.
                let commit = tx.store.read(BLOB, &entry.blob)?;
                if commit.len() != 40 || !commit.iter().all(u8::is_ascii_hexdigit) {
                    return Err(Error::InvalidArgument(format!(
                        "submodule `{path}` does not point at a Git commit id"
                    )));
                }
                let commit = String::from_utf8_lossy(&commit);
                writeln!(out, "M 160000 {commit} {}", quote_path(path)).map_err(io)?;
                continue;
            }
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
    for (name, tip) in refs {
        let target = commit_marks
            .get(tip)
            .map(|m| format!(":{m}"))
            .or_else(|| known(tip));
        if let Some(target) = target {
            writeln!(out, "reset {name}\nfrom {target}\n").map_err(io)?;
        }
    }
    writeln!(out, "done").map_err(io)?;
    Ok(Exported {
        stats: ExportStats {
            commits: commits.len(),
            blobs: blob_marks.len(),
        },
        marks: commit_marks.into_iter().map(|(id, m)| (m, id)).collect(),
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
