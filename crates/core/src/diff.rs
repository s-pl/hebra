//! Differences between two trees.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::error::Result;
use crate::model::Tree;
use crate::store::{BLOB, Store};
use crate::wc::is_binary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

impl FileStatus {
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: FileStatus,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    /// Unified diff, when requested and the file is text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

/// Lists changed files only (cheap: compares blob ids).
#[must_use]
pub fn changed_paths(from: &Tree, to: &Tree) -> Vec<(String, FileStatus)> {
    let paths: BTreeSet<&String> = from.files.keys().chain(to.files.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| match (from.files.get(path), to.files.get(path)) {
            (None, Some(_)) => Some((path.clone(), FileStatus::Added)),
            (Some(_), None) => Some((path.clone(), FileStatus::Deleted)),
            (Some(a), Some(b)) if a != b => Some((path.clone(), FileStatus::Modified)),
            _ => None,
        })
        .collect()
}

/// Full diff with line counts and, optionally, unified patches.
pub fn diff_trees(
    store: &Store,
    from: &Tree,
    to: &Tree,
    with_patch: bool,
) -> Result<Vec<FileDiff>> {
    let mut out = Vec::new();
    for (path, status) in changed_paths(from, to) {
        let old = match from.files.get(&path) {
            Some(e) => store.read(BLOB, &e.blob)?,
            None => Vec::new(),
        };
        let new = match to.files.get(&path) {
            Some(e) => store.read(BLOB, &e.blob)?,
            None => Vec::new(),
        };
        let binary = is_binary(&old) || is_binary(&new);
        let (old_text, new_text) = (String::from_utf8_lossy(&old), String::from_utf8_lossy(&new));
        let (mut additions, mut deletions, mut patch) = (0, 0, None);
        if !binary {
            let mut options = diffy::DiffOptions::new();
            options
                .set_original_filename(format!("a/{path}"))
                .set_modified_filename(format!("b/{path}"));
            let p = options.create_patch(&old_text, &new_text);
            for hunk in p.hunks() {
                for line in hunk.lines() {
                    match line {
                        diffy::Line::Insert(_) => additions += 1,
                        diffy::Line::Delete(_) => deletions += 1,
                        diffy::Line::Context(_) => {}
                    }
                }
            }
            if with_patch {
                patch = Some(p.to_string());
            }
        }
        out.push(FileDiff {
            path,
            status,
            additions,
            deletions,
            binary,
            patch,
        });
    }
    Ok(out)
}
