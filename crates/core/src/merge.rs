//! Three-way tree merge. Conflicts are returned as data, never as errors.

use std::collections::BTreeSet;

use crate::error::Result;
use crate::model::{Conflict, ConflictKind, FileEntry, Tree};
use crate::store::{BLOB, Store};
use crate::wc::is_binary;

/// Conflict marker that starts a conflicted region (diff3 style).
pub const CONFLICT_START: &str = "<<<<<<<";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    pub tree: Tree,
    pub conflicts: Vec<Conflict>,
}

pub fn merge_trees(store: &Store, base: &Tree, ours: &Tree, theirs: &Tree) -> Result<MergeResult> {
    let paths: BTreeSet<&String> = base
        .files
        .keys()
        .chain(ours.files.keys())
        .chain(theirs.files.keys())
        .collect();

    let mut tree = Tree::default();
    let mut conflicts = Vec::new();
    for path in paths {
        let b = base.files.get(path);
        let o = ours.files.get(path);
        let t = theirs.files.get(path);
        let merged = if o == t || t == b {
            o.copied()
        } else if o == b {
            t.copied()
        } else {
            match (o, t) {
                (Some(o), Some(t)) => {
                    let (entry, conflict) = merge_file(store, b, o, t)?;
                    if let Some(kind) = conflict {
                        conflicts.push(Conflict {
                            path: path.clone(),
                            kind,
                        });
                    }
                    Some(entry)
                }
                // Modified on one side, deleted on the other: keep the
                // modification so no work is lost, and flag it.
                (Some(kept), None) | (None, Some(kept)) => {
                    conflicts.push(Conflict {
                        path: path.clone(),
                        kind: ConflictKind::ModifyDelete,
                    });
                    Some(*kept)
                }
                (None, None) => None,
            }
        };
        if let Some(entry) = merged {
            tree.files.insert(path.clone(), entry);
        }
    }
    Ok(MergeResult { tree, conflicts })
}

fn merge_file(
    store: &Store,
    base: Option<&FileEntry>,
    ours: &FileEntry,
    theirs: &FileEntry,
) -> Result<(FileEntry, Option<ConflictKind>)> {
    let executable = if base.is_some_and(|b| b.executable == ours.executable) {
        theirs.executable
    } else {
        ours.executable
    };
    if ours.blob == theirs.blob {
        return Ok((
            FileEntry {
                blob: ours.blob,
                executable,
            },
            None,
        ));
    }
    let base_data = match base {
        Some(b) => store.read(BLOB, &b.blob)?,
        None => Vec::new(),
    };
    let ours_data = store.read(BLOB, &ours.blob)?;
    let theirs_data = store.read(BLOB, &theirs.blob)?;
    if is_binary(&base_data) || is_binary(&ours_data) || is_binary(&theirs_data) {
        return Ok((
            FileEntry {
                blob: ours.blob,
                executable,
            },
            Some(ConflictKind::Binary),
        ));
    }
    let (data, conflict) = match diffy::merge_bytes(&base_data, &ours_data, &theirs_data) {
        Ok(clean) => (clean, None),
        Err(with_markers) => (with_markers, Some(ConflictKind::Content)),
    };
    let blob = store.write(BLOB, &data)?;
    Ok((FileEntry { blob, executable }, conflict))
}

/// Whether a file still contains conflict markers.
#[must_use]
pub fn has_conflict_markers(data: &[u8]) -> bool {
    data.split(|&b| b == b'\n')
        .any(|line| line.starts_with(CONFLICT_START.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(store: &Store, files: &[(&str, &str)]) -> Tree {
        let mut tree = Tree::default();
        for (path, content) in files {
            let blob = store.write(BLOB, content.as_bytes()).unwrap();
            tree.files.insert(
                (*path).to_owned(),
                FileEntry {
                    blob,
                    executable: false,
                },
            );
        }
        tree
    }

    fn content(store: &Store, tree: &Tree, path: &str) -> String {
        String::from_utf8(store.read(BLOB, &tree.files[path].blob).unwrap()).unwrap()
    }

    #[test]
    fn merges_independent_edits_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let base = tree(&store, &[("a.txt", "1\n2\n3\n"), ("gone.txt", "x\n")]);
        let ours = tree(
            &store,
            &[
                ("a.txt", "ONE\n2\n3\n"),
                ("gone.txt", "x\n"),
                ("new.txt", "n\n"),
            ],
        );
        let theirs = tree(&store, &[("a.txt", "1\n2\nTHREE\n")]);
        let result = merge_trees(&store, &base, &ours, &theirs).unwrap();
        assert!(result.conflicts.is_empty());
        assert_eq!(content(&store, &result.tree, "a.txt"), "ONE\n2\nTHREE\n");
        assert!(!result.tree.files.contains_key("gone.txt"));
        assert!(result.tree.files.contains_key("new.txt"));
    }

    #[test]
    fn records_conflicts_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let base = tree(&store, &[("a.txt", "1\n"), ("b.txt", "b\n")]);
        let ours = tree(&store, &[("a.txt", "ours\n"), ("b.txt", "changed\n")]);
        let theirs = tree(&store, &[("a.txt", "theirs\n")]);
        let result = merge_trees(&store, &base, &ours, &theirs).unwrap();
        assert_eq!(result.conflicts.len(), 2);
        assert!(has_conflict_markers(
            content(&store, &result.tree, "a.txt").as_bytes()
        ));
        assert_eq!(content(&store, &result.tree, "b.txt"), "changed\n");
    }
}
