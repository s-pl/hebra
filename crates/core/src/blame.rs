//! Who wrote each line: Hebra's `git blame`, with the agent, task and prompt
//! behind every line as well as the person.
//!
//! Starts with every line unexplained and walks history newest first. At each
//! commit, lines that also exist in a parent are passed on to that parent;
//! the ones left were written by this commit. Renames are followed.

use std::collections::{BTreeMap, BinaryHeap, HashMap};

use crate::error::{Error, Result};
use crate::ids::ObjectId;
use crate::model::{Commit, Tree};
use crate::rename;
use crate::repo::Tx;
use crate::store::{BLOB, COMMIT, Store};
use crate::wc::is_binary;

/// One line of a file, with the commit that wrote it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BlameLine {
    /// Line number in the file, from 1.
    pub line: usize,
    pub commit: ObjectId,
    /// The path the line had in that commit.
    pub path: String,
    pub text: String,
}

/// Lines still to explain: their number in the original file, by their index
/// in this version.
type Pending = BTreeMap<usize, usize>;

/// For each line of `new`, its index in `old`, if it was kept.
fn kept_lines(old: &str, new: &str) -> HashMap<usize, usize> {
    let patch = diffy::DiffOptions::new()
        .set_context_len(0)
        .create_patch(old, new);
    let mut map = HashMap::new();
    let (old_lines, new_lines) = (old.lines().count(), new.lines().count());
    let (mut o, mut n) = (0usize, 0usize);
    for hunk in patch.hunks() {
        let old_start = hunk.old_range().start().saturating_sub(1);
        let new_start = hunk.new_range().start().saturating_sub(1);
        // An empty range points at the line before it.
        let old_start = if hunk.old_range().is_empty() {
            hunk.old_range().start()
        } else {
            old_start
        };
        let new_start = if hunk.new_range().is_empty() {
            hunk.new_range().start()
        } else {
            new_start
        };
        while n < new_start && o < old_start {
            map.insert(n, o);
            n += 1;
            o += 1;
        }
        o = old_start + hunk.old_range().len();
        n = new_start + hunk.new_range().len();
    }
    while n < new_lines && o < old_lines {
        map.insert(n, o);
        n += 1;
        o += 1;
    }
    map
}

/// What blame needs from a history. Repositories answer from their index;
/// a server, which has only objects, works it out.
pub trait History {
    fn objects(&self) -> &Store;
    fn get_commit(&mut self, id: &ObjectId) -> Result<Commit>;
    /// A number greater than any of the commit's parents have.
    fn depth(&mut self, id: &ObjectId) -> Result<u64>;
}

impl History for Tx<'_> {
    fn objects(&self) -> &Store {
        self.store
    }

    fn get_commit(&mut self, id: &ObjectId) -> Result<Commit> {
        self.commit(id)
    }

    fn depth(&mut self, id: &ObjectId) -> Result<u64> {
        Ok(self.node(id)?.generation)
    }
}

/// History from objects alone.
pub struct StoreHistory<'a> {
    store: &'a Store,
    depths: HashMap<ObjectId, u64>,
}

impl<'a> StoreHistory<'a> {
    #[must_use]
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            depths: HashMap::new(),
        }
    }
}

impl History for StoreHistory<'_> {
    fn objects(&self) -> &Store {
        self.store
    }

    fn get_commit(&mut self, id: &ObjectId) -> Result<Commit> {
        self.store.read_json(COMMIT, id)
    }

    fn depth(&mut self, id: &ObjectId) -> Result<u64> {
        let mut pending = vec![*id];
        while let Some(&top) = pending.last() {
            if self.depths.contains_key(&top) {
                pending.pop();
                continue;
            }
            let commit: Commit = self.store.read_json(COMMIT, &top)?;
            let missing: Vec<ObjectId> = commit
                .parents
                .iter()
                .filter(|p| !self.depths.contains_key(p))
                .copied()
                .collect();
            if missing.is_empty() {
                let depth = commit
                    .parents
                    .iter()
                    .filter_map(|p| self.depths.get(p))
                    .map(|d| d + 1)
                    .max()
                    .unwrap_or(0);
                self.depths.insert(top, depth);
                pending.pop();
            } else {
                pending.extend(missing);
            }
        }
        Ok(self.depths.get(id).copied().unwrap_or(0))
    }
}

fn text_of(store: &Store, tree: &Tree, path: &str) -> Result<Option<String>> {
    let Some(entry) = tree.files.get(path) else {
        return Ok(None);
    };
    if !entry.is_file() {
        return Ok(None);
    }
    let data = store.read(BLOB, &entry.blob)?;
    Ok((!is_binary(&data)).then(|| String::from_utf8_lossy(&data).into_owned()))
}

/// The path `path` of `commit_tree` had in `parent_tree`, following a rename.
fn path_in_parent(
    store: &Store,
    commit_tree: &Tree,
    parent_tree: &Tree,
    path: &str,
) -> Result<Option<String>> {
    if parent_tree.files.contains_key(path) {
        return Ok(Some(path.to_owned()));
    }
    Ok(rename::detect(store, parent_tree, commit_tree)?
        .into_iter()
        .find(|r| r.to == path)
        .map(|r| r.from))
}

fn tree_at(history: &mut impl History, id: &ObjectId) -> Result<Tree> {
    let tree = history.get_commit(id)?.tree;
    history.objects().read_tree(&tree)
}

/// Attributes every line of `path` as of `start`.
pub fn blame(history: &mut impl History, start: ObjectId, path: &str) -> Result<Vec<BlameLine>> {
    let tree = tree_at(history, &start)?;
    let text = text_of(history.objects(), &tree, path)?.ok_or_else(|| {
        Error::InvalidArgument(format!("`{path}` is not a text file in that revision"))
    })?;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut pending: Pending = (0..lines.len()).map(|i| (i, i)).collect();
    let mut result: BTreeMap<usize, (ObjectId, String)> = BTreeMap::new();

    // Newest first, so a commit is reached only after all its descendants
    // have passed their lines on to it.
    let mut work: HashMap<(ObjectId, String), Pending> = HashMap::new();
    let mut queue: BinaryHeap<(u64, ObjectId, String)> = BinaryHeap::new();
    let depth = history.depth(&start)?;
    work.insert((start, path.to_owned()), std::mem::take(&mut pending));
    queue.push((depth, start, path.to_owned()));

    while let Some((_, id, path)) = queue.pop() {
        let Some(mut lines_here) = work.remove(&(id, path.clone())) else {
            continue;
        };
        let commit = history.get_commit(&id)?;
        let tree = tree_at(history, &id)?;
        let Some(text) = text_of(history.objects(), &tree, &path)? else {
            continue;
        };
        for parent in commit.parents.clone() {
            if lines_here.is_empty() {
                break;
            }
            let parent_tree = tree_at(history, &parent)?;
            let store = history.objects();
            let Some(parent_path) = path_in_parent(store, &tree, &parent_tree, &path)? else {
                continue;
            };
            let same = tree.files.get(&path).map(|e| e.blob)
                == parent_tree.files.get(&parent_path).map(|e| e.blob);
            let Some(parent_text) = text_of(store, &parent_tree, &parent_path)? else {
                continue;
            };
            let kept = if same {
                None
            } else {
                Some(kept_lines(&parent_text, &text))
            };
            let mut passed = Pending::new();
            lines_here.retain(|index, original| {
                let target = match &kept {
                    None => Some(*index),
                    Some(map) => map.get(index).copied(),
                };
                match target {
                    Some(i) => {
                        passed.insert(i, *original);
                        false
                    }
                    None => true,
                }
            });
            if !passed.is_empty() {
                let depth = history.depth(&parent)?;
                let entry = work.entry((parent, parent_path.clone())).or_default();
                if entry.is_empty() {
                    queue.push((depth, parent, parent_path));
                }
                entry.extend(passed);
            }
        }
        // What no parent explains was written here.
        for original in lines_here.into_values() {
            result.insert(original, (id, path.clone()));
        }
    }
    Ok(lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let (commit, path) = result
                .get(&i)
                .cloned()
                .unwrap_or_else(|| (start, path.to_owned()));
            BlameLine {
                line: i + 1,
                commit,
                path,
                text,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_kept_lines_across_edits() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nX\nc\nd\ne\n";
        let map = kept_lines(old, new);
        assert_eq!(map.get(&0), Some(&0));
        assert_eq!(map.get(&1), None);
        assert_eq!(map.get(&2), Some(&2));
        assert_eq!(map.get(&3), Some(&3));
        assert_eq!(map.get(&4), None);

        let inserted = kept_lines("a\nb\n", "new\na\nb\n");
        assert_eq!(inserted.get(&0), None);
        assert_eq!(inserted.get(&1), Some(&0));
        assert_eq!(inserted.get(&2), Some(&1));

        let removed = kept_lines("a\nb\nc\n", "a\nc\n");
        assert_eq!(removed.get(&1), Some(&2));
    }
}
