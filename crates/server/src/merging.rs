//! Reviews: someone (usually an agent) asks for a bookmark to be merged into
//! another (usually `main`); a person with permission approves and the
//! server performs the merge itself.

use std::collections::{HashSet, VecDeque};

use hebra_core::ObjectId;
use hebra_core::diff;
use hebra_core::merge::merge_trees;
use hebra_core::model::{Actor, Commit, Provenance, Tree};
use hebra_core::store::{COMMIT, Store, TREE};
use serde::Serialize;

/// What merging `source` into `target` would do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Mergeability {
    /// Everything in `source` is already in `target`.
    UpToDate,
    /// `target` can simply move forward to `source`.
    FastForward,
    /// A merge commit is needed and has no conflicts.
    Clean,
    /// A merge commit would contain conflicts in these files.
    Conflicts { paths: Vec<String> },
}

fn ancestors(store: &Store, start: ObjectId) -> hebra_core::Result<HashSet<ObjectId>> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        if seen.insert(id) {
            let c: Commit = store.read_json(COMMIT, &id)?;
            queue.extend(c.parents);
        }
    }
    Ok(seen)
}

/// Nearest common ancestor (breadth-first from `b`).
pub fn merge_base(store: &Store, a: ObjectId, b: ObjectId) -> hebra_core::Result<Option<ObjectId>> {
    let of_a = ancestors(store, a)?;
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([b]);
    while let Some(id) = queue.pop_front() {
        if of_a.contains(&id) {
            return Ok(Some(id));
        }
        if seen.insert(id) {
            let c: Commit = store.read_json(COMMIT, &id)?;
            queue.extend(c.parents);
        }
    }
    Ok(None)
}

fn tree_of(store: &Store, commit: Option<ObjectId>) -> hebra_core::Result<Tree> {
    match commit {
        Some(id) => {
            let c: Commit = store.read_json(COMMIT, &id)?;
            store.read_json(TREE, &c.tree)
        }
        None => Ok(Tree::default()),
    }
}

pub struct Plan {
    pub mergeability: Mergeability,
    pub base: Option<ObjectId>,
    /// The merged tree when a merge commit is needed and clean.
    pub tree: Option<Tree>,
}

pub fn plan(store: &Store, target: ObjectId, source: ObjectId) -> hebra_core::Result<Plan> {
    let target_ancestors = ancestors(store, target)?;
    if target_ancestors.contains(&source) {
        return Ok(Plan {
            mergeability: Mergeability::UpToDate,
            base: Some(source),
            tree: None,
        });
    }
    if ancestors(store, source)?.contains(&target) {
        return Ok(Plan {
            mergeability: Mergeability::FastForward,
            base: Some(target),
            tree: None,
        });
    }
    let base = merge_base(store, target, source)?;
    let result = merge_trees(
        store,
        &tree_of(store, base)?,
        &tree_of(store, Some(target))?,
        &tree_of(store, Some(source))?,
    )?;
    if result.conflicts.is_empty() {
        Ok(Plan {
            mergeability: Mergeability::Clean,
            base,
            tree: Some(result.tree),
        })
    } else {
        Ok(Plan {
            mergeability: Mergeability::Conflicts {
                paths: result.conflicts.into_iter().map(|c| c.path).collect(),
            },
            base,
            tree: None,
        })
    }
}

/// Files the review would bring in: source against the merge base.
pub fn changes(
    store: &Store,
    base: Option<ObjectId>,
    source: ObjectId,
) -> hebra_core::Result<Vec<diff::FileDiff>> {
    diff::diff_trees(
        store,
        &tree_of(store, base)?,
        &tree_of(store, Some(source))?,
        true,
    )
}

/// Commits in `source` that `target` does not have yet, newest first.
pub fn commits(
    store: &Store,
    target: ObjectId,
    source: ObjectId,
    limit: usize,
) -> hebra_core::Result<Vec<(ObjectId, Commit)>> {
    let known = ancestors(store, target)?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([source]);
    while let Some(id) = queue.pop_front() {
        if known.contains(&id) || !seen.insert(id) || out.len() >= limit {
            continue;
        }
        let c: Commit = store.read_json(COMMIT, &id)?;
        queue.extend(c.parents.iter().copied());
        out.push((id, c));
    }
    out.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
    Ok(out)
}

/// Writes the merge commit for a clean plan, authored by the approver.
#[allow(clippy::too_many_arguments)]
pub fn write_merge(
    store: &Store,
    target: ObjectId,
    source: ObjectId,
    tree: &Tree,
    title: &str,
    source_name: &str,
    target_name: &str,
    approver: &Actor,
) -> hebra_core::Result<ObjectId> {
    let tree_id = store.write_json(TREE, tree)?;
    let commit = Commit {
        change_id: hebra_core::ChangeId::random(),
        parents: vec![target, source],
        predecessors: Vec::new(),
        tree: tree_id,
        description: format!("Merge {source_name} into {target_name}\n\n{title}"),
        author: approver.clone(),
        timestamp: jiff::Timestamp::now().to_string(),
        provenance: Provenance {
            task: Some(title.to_owned()),
            ..Provenance::default()
        },
        conflicts: Vec::new(),
    };
    store.write_json(COMMIT, &commit)
}
