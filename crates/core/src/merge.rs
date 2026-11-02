//! Three-way tree merge. Conflicts are returned as data, never as errors.

use std::collections::BTreeSet;

use crate::error::Result;
use crate::model::{Conflict, ConflictKind, FileEntry, Tree};
use crate::rename;
use crate::store::{BLOB, Store};
use crate::structural;
use crate::wc::is_binary;

/// Conflict marker that starts a conflicted region (diff3 style).
pub const CONFLICT_START: &str = "<<<<<<<";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    pub tree: Tree,
    pub conflicts: Vec<Conflict>,
}

/// Three-way merge that follows renames, like Git: if one side renamed a
/// file and the other changed it, the change lands in the renamed file.
pub fn merge_trees(store: &Store, base: &Tree, ours: &Tree, theirs: &Tree) -> Result<MergeResult> {
    let ours_renames = rename::detect(store, base, ours)?;
    let theirs_renames = rename::detect(store, base, theirs)?;
    if ours_renames.is_empty() && theirs_renames.is_empty() {
        return merge_paths(store, base, ours, theirs);
    }
    let (mut base, mut ours, mut theirs) = (base.clone(), ours.clone(), theirs.clone());
    let mut conflicts = Vec::new();
    for r in &ours_renames {
        match theirs_renames.iter().find(|t| t.from == r.from) {
            Some(t) if t.to == r.to => {
                // Same rename on both sides: an ordinary merge at the new path.
                move_entry(&mut base, &r.from, &r.to);
            }
            Some(t) => conflicts.push(Conflict {
                path: t.to.clone(),
                kind: ConflictKind::Rename,
            }),
            None => {
                move_entry(&mut base, &r.from, &r.to);
                move_entry(&mut theirs, &r.from, &r.to);
            }
        }
    }
    for t in &theirs_renames {
        if !ours_renames.iter().any(|r| r.from == t.from) {
            move_entry(&mut base, &t.from, &t.to);
            move_entry(&mut ours, &t.from, &t.to);
        }
    }
    let mut merged = merge_paths(store, &base, &ours, &theirs)?;
    merged.conflicts.extend(conflicts);
    Ok(merged)
}

fn move_entry(tree: &mut Tree, from: &str, to: &str) {
    if let Some(entry) = tree.files.remove(from) {
        tree.files.insert(to.to_owned(), entry);
    }
}

/// Merges path by path.
fn merge_paths(store: &Store, base: &Tree, ours: &Tree, theirs: &Tree) -> Result<MergeResult> {
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
                    let (entry, conflict) = merge_file(store, path, b, o, t)?;
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
    path: &str,
    base: Option<&FileEntry>,
    ours: &FileEntry,
    theirs: &FileEntry,
) -> Result<(FileEntry, Option<ConflictKind>)> {
    let executable = if base.is_some_and(|b| b.executable == ours.executable) {
        theirs.executable
    } else {
        ours.executable
    };
    if ours.blob == theirs.blob && ours.is_file() == theirs.is_file() {
        return Ok((
            FileEntry {
                executable,
                ..*ours
            },
            None,
        ));
    }
    // Symlinks and submodules have no lines to merge: keep ours.
    if !ours.is_file() || !theirs.is_file() || base.is_some_and(|b| !b.is_file()) {
        return Ok((*ours, Some(ConflictKind::Binary)));
    }
    let base_data = match base {
        Some(b) => store.read(BLOB, &b.blob)?,
        None => Vec::new(),
    };
    let ours_data = store.read(BLOB, &ours.blob)?;
    let theirs_data = store.read(BLOB, &theirs.blob)?;
    if is_binary(&base_data) || is_binary(&ours_data) || is_binary(&theirs_data) {
        return Ok((
            FileEntry::file(ours.blob, executable),
            Some(ConflictKind::Binary),
        ));
    }
    let (data, conflict) = match diffy::merge_bytes(&base_data, &ours_data, &theirs_data) {
        Ok(clean) => (clean, None),
        // Lines conflict: try declaration by declaration before giving up.
        Err(with_markers) => match structural_merge(path, &base_data, &ours_data, &theirs_data) {
            Some(clean) => (clean, None),
            None => (with_markers, Some(ConflictKind::Content)),
        },
    };
    let blob = store.write(BLOB, &data)?;
    Ok((FileEntry::file(blob, executable), conflict))
}

fn structural_merge(path: &str, base: &[u8], ours: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    if !structural::supports(path) {
        return None;
    }
    let text = |b: &[u8]| std::str::from_utf8(b).ok().map(str::to_owned);
    structural::merge(path, &text(base)?, &text(ours)?, &text(theirs)?).map(String::into_bytes)
}

/// Which side wins a conflicted region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// Replaces each conflicted region with one of its sides.
#[must_use]
pub fn settle_markers(data: &[u8], side: Side) -> Vec<u8> {
    #[derive(PartialEq)]
    enum State {
        Plain,
        Ours,
        Base,
        Theirs,
    }
    let mut state = State::Plain;
    let mut out = Vec::with_capacity(data.len());
    for line in data.split_inclusive(|&b| b == b'\n') {
        if line.starts_with(b"<<<<<<<") && state == State::Plain {
            state = State::Ours;
        } else if line.starts_with(b"|||||||") && state == State::Ours {
            state = State::Base;
        } else if line.starts_with(b"=======") && matches!(state, State::Ours | State::Base) {
            state = State::Theirs;
        } else if line.starts_with(b">>>>>>>") && state == State::Theirs {
            state = State::Plain;
        } else {
            let keep = match state {
                State::Plain => true,
                State::Ours => side == Side::Ours,
                State::Theirs => side == Side::Theirs,
                State::Base => false,
            };
            if keep {
                out.extend_from_slice(line);
            }
        }
    }
    out
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
            tree.files
                .insert((*path).to_owned(), FileEntry::file(blob, false));
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
    fn a_change_follows_a_rename_on_the_other_side() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let body: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let changed = body.replace("line 5\n", "line five\n");
        let base = tree(&store, &[("old.rs", &body)]);
        let renamed = tree(&store, &[("new.rs", &body)]);
        let edited = tree(&store, &[("old.rs", &changed)]);
        for (ours, theirs) in [(&renamed, &edited), (&edited, &renamed)] {
            let result = merge_trees(&store, &base, ours, theirs).unwrap();
            assert!(result.conflicts.is_empty(), "{:?}", result.conflicts);
            assert!(!result.tree.files.contains_key("old.rs"));
            assert_eq!(content(&store, &result.tree, "new.rs"), changed);
        }
    }

    #[test]
    fn renaming_to_two_names_is_recorded_and_keeps_both() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let body: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let base = tree(&store, &[("old.rs", &body)]);
        let ours = tree(&store, &[("mine.rs", &body)]);
        let theirs = tree(&store, &[("yours.rs", &body)]);
        let result = merge_trees(&store, &base, &ours, &theirs).unwrap();
        assert_eq!(result.conflicts[0].kind, ConflictKind::Rename);
        assert!(result.tree.files.contains_key("mine.rs"));
        assert!(result.tree.files.contains_key("yours.rs"));
    }

    #[test]
    fn conflicted_regions_settle_to_either_side() {
        let text = b"a\n<<<<<<< ours\nmine\n||||||| base\nold\n=======\nyours\n>>>>>>> theirs\nz\n";
        assert_eq!(settle_markers(text, Side::Ours), b"a\nmine\nz\n");
        assert_eq!(settle_markers(text, Side::Theirs), b"a\nyours\nz\n");
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
