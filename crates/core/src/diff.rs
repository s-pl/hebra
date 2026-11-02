//! Differences between two trees.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::model::Tree;
use crate::rename;
use crate::store::{BLOB, Store};
use crate::wc::is_binary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl FileStatus {
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: FileStatus,
    /// The path it had before, when it was renamed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
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

/// The `diff --git` lines Git puts before a file's patch. Git needs them to
/// tell a new or deleted file from a change to an existing one.
#[must_use]
pub fn git_header(file: &FileDiff) -> String {
    let old = file.renamed_from.as_deref().unwrap_or(&file.path);
    let mut out = format!("diff --git a/{old} b/{}\n", file.path);
    match file.status {
        FileStatus::Added => out.push_str("new file mode 100644\n"),
        FileStatus::Deleted => out.push_str("deleted file mode 100644\n"),
        FileStatus::Renamed => {
            out.push_str(&format!("rename from {old}\nrename to {}\n", file.path));
        }
        FileStatus::Modified => {}
    }
    out
}

/// How to show a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffOptions {
    pub with_patch: bool,
    /// Lines of context around each change.
    pub context: usize,
    /// Show renames as renames instead of a delete and an add.
    pub renames: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            with_patch: true,
            context: 3,
            renames: true,
        }
    }
}

/// Full diff with line counts and, optionally, unified patches.
pub fn diff_trees(
    store: &Store,
    from: &Tree,
    to: &Tree,
    with_patch: bool,
) -> Result<Vec<FileDiff>> {
    diff_with(
        store,
        from,
        to,
        DiffOptions {
            with_patch,
            ..DiffOptions::default()
        },
    )
}

/// Full diff, with options.
pub fn diff_with(
    store: &Store,
    from: &Tree,
    to: &Tree,
    opts: DiffOptions,
) -> Result<Vec<FileDiff>> {
    let renames: BTreeMap<String, String> = if opts.renames {
        rename::detect(store, from, to)?
            .into_iter()
            .map(|r| (r.to, r.from))
            .collect()
    } else {
        BTreeMap::new()
    };
    let moved: BTreeSet<&String> = renames.values().collect();
    let mut out = Vec::new();
    for (path, status) in changed_paths(from, to) {
        if moved.contains(&path) {
            continue;
        }
        let renamed_from = renames.get(&path).cloned();
        let old_path = renamed_from.as_ref().unwrap_or(&path);
        let old = match from.files.get(old_path) {
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
            let original = if status == FileStatus::Added {
                "/dev/null".to_owned()
            } else {
                format!("a/{old_path}")
            };
            let modified = if status == FileStatus::Deleted {
                "/dev/null".to_owned()
            } else {
                format!("b/{path}")
            };
            options
                .set_context_len(opts.context)
                .set_original_filename(original)
                .set_modified_filename(modified);
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
            if opts.with_patch {
                patch = Some(p.to_string());
            }
        }
        let status = if renamed_from.is_some() {
            FileStatus::Renamed
        } else {
            status
        };
        out.push(FileDiff {
            path,
            status,
            renamed_from,
            additions,
            deletions,
            binary,
            patch,
        });
    }
    Ok(out)
}
