//! Rewriting history: rebase, cherry-pick, revert, squash, split, abandon,
//! reset and stash. Nothing here can stop half-way: a conflict is recorded in
//! the commit it happens in and the operation finishes, so there is no
//! "rebase in progress" to continue or abort. `hb undo` takes it all back.
//!
//! Rewritten commits keep their change id, so the same change can be followed
//! across versions; copies (cherry-pick) get a new one.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::error::{Error, Result};
use crate::ids::{ChangeId, ObjectId};
use crate::merge::{self, has_conflict_markers};
use crate::model::{Commit, Conflict, ConflictKind, Tree};
use crate::repo::{Tx, now};
use crate::store::BLOB;

/// Paths named on the command line: files, directories or globs, relative to
/// the workspace root, with the syntax of `.gitignore` lines.
#[derive(Debug, Clone, Default)]
pub struct PathSpec {
    matcher: Option<ignore::gitignore::Gitignore>,
}

impl PathSpec {
    pub fn new(specs: &[String]) -> Result<Self> {
        if specs.is_empty() {
            return Ok(Self::default());
        }
        let mut builder = ignore::gitignore::GitignoreBuilder::new("");
        for spec in specs {
            let line = spec.trim_start_matches("./");
            let line = if line == "." || line.is_empty() {
                "/**"
            } else {
                line
            };
            // Anchored, so `lib.rs` means that path and not any file so named.
            let anchored = if line.starts_with('/') || line.starts_with('*') {
                line.to_owned()
            } else {
                format!("/{line}")
            };
            builder
                .add_line(None, &anchored)
                .map_err(|e| Error::InvalidArgument(format!("invalid path `{spec}`: {e}")))?;
        }
        let matcher = builder
            .build()
            .map_err(|e| Error::InvalidArgument(format!("invalid paths: {e}")))?;
        Ok(Self {
            matcher: Some(matcher),
        })
    }

    #[must_use]
    pub fn is_all(&self) -> bool {
        self.matcher.is_none()
    }

    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.matcher
            .as_ref()
            .is_none_or(|m| m.matched_path_or_any_parents(path, false).is_ignore())
    }
}

/// `base`, with the matching paths as they are in `other`.
#[must_use]
pub fn take_paths(base: &Tree, other: &Tree, spec: &PathSpec) -> Tree {
    let mut out = base.clone();
    out.files.retain(|path, _| !spec.matches(path));
    for (path, entry) in &other.files {
        if spec.matches(path) {
            out.files.insert(path.clone(), *entry);
        }
    }
    out
}

/// One step of an interactive rebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Pick(ObjectId),
    /// Keep the change with a new description.
    Reword(ObjectId, String),
    /// Fold into the previous step, keeping both descriptions.
    Squash(ObjectId),
    /// Fold into the previous step, dropping this description.
    Fixup(ObjectId),
    Drop(ObjectId),
}

/// Whether a commit counts as a merge-free chain step for `rebase`.
fn merged_description(first: &str, second: &str) -> String {
    match (first.trim().is_empty(), second.trim().is_empty()) {
        (true, _) => second.to_owned(),
        (_, true) => first.to_owned(),
        _ => format!("{}\n\n{}", first.trim_end(), second.trim_end()),
    }
}

impl Tx<'_> {
    /// Commits that are visible descendants of `id`, parents first.
    pub fn descendants(&mut self, id: ObjectId) -> Result<Vec<ObjectId>> {
        let floor = self.node(&id)?.generation;
        let mut pending = self.tips();
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current) {
                continue;
            }
            let node = self.node(&current)?;
            if node.generation > floor {
                candidates.push((node.generation, current));
                pending.extend(node.parents);
            }
        }
        candidates.sort_unstable();
        let mut inside = HashSet::from([id]);
        let mut out = Vec::new();
        for (_, candidate) in candidates {
            if self
                .node(&candidate)?
                .parents
                .iter()
                .any(|p| inside.contains(p))
            {
                inside.insert(candidate);
                out.push(candidate);
            }
        }
        Ok(out)
    }

    /// Ancestors of `to` (itself included) that are not ancestors of `from`,
    /// parents first: Git's `from..to`.
    pub fn range(&mut self, from: Option<ObjectId>, to: ObjectId) -> Result<Vec<ObjectId>> {
        let excluded = match from {
            Some(id) => self.ancestors(id)?,
            None => HashSet::from([self.root_commit]),
        };
        let mut pending = vec![to];
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        while let Some(id) = pending.pop() {
            if excluded.contains(&id) || !seen.insert(id) {
                continue;
            }
            let node = self.node(&id)?;
            out.push((node.generation, id));
            pending.extend(node.parents);
        }
        out.sort_unstable();
        Ok(out.into_iter().map(|(_, id)| id).collect())
    }

    /// The changes of `id` applied on top of `parents`: the tree and any
    /// conflicts. `mainline` picks which parent of a merge counts as base.
    pub fn replayed_tree(
        &mut self,
        id: ObjectId,
        parents: &[ObjectId],
        mainline: Option<usize>,
    ) -> Result<(ObjectId, Tree, Vec<Conflict>)> {
        let commit = self.commit(&id)?;
        let base = self.change_base(&commit, mainline)?;
        let (ours, mut conflicts) = self.parents_tree(parents)?;
        let theirs = self.tree(&commit.tree)?;
        let merged = merge::merge_trees(self.store, &base, &ours, &theirs)?;
        conflicts.extend(merged.conflicts);
        // Conflicts this commit already had travel with it while unresolved.
        for old in &commit.conflicts {
            if !conflicts.iter().any(|c| c.path == old.path)
                && let Some(entry) = merged.tree.files.get(&old.path)
                && (old.kind != ConflictKind::Content
                    || has_conflict_markers(&self.store.read(BLOB, &entry.blob)?))
            {
                conflicts.push(old.clone());
            }
        }
        let tree = self.store.write_tree(&merged.tree)?;
        Ok((tree, merged.tree, conflicts))
    }

    /// What `commit` was built on.
    fn change_base(&mut self, commit: &Commit, mainline: Option<usize>) -> Result<Tree> {
        match mainline {
            Some(n) => {
                let parent = n
                    .checked_sub(1)
                    .and_then(|i| commit.parents.get(i))
                    .copied()
                    .ok_or_else(|| {
                        Error::InvalidArgument(format!("the commit has no parent number {n}"))
                    })?;
                self.commit_tree(&parent)
            }
            None if commit.parents.len() > 1 => Err(Error::InvalidArgument(
                "this commit is a merge: choose a side with --mainline".to_owned(),
            )),
            None => Ok(self.parents_tree(&commit.parents)?.0),
        }
    }

    /// A new version of `id` on top of `parents`, same change id. The author
    /// and the date the work was written stay.
    fn move_commit(&mut self, id: ObjectId, parents: Vec<ObjectId>) -> Result<ObjectId> {
        let old = self.commit(&id)?;
        if old.parents == parents {
            return Ok(id);
        }
        let (tree, _, conflicts) = if old.parents.len() > 1 {
            self.replay_merge(&old, &parents)?
        } else {
            self.replayed_tree(id, &parents, None)?
        };
        let mut new = old.clone();
        new.parents = parents;
        new.tree = tree;
        new.conflicts = conflicts;
        new.predecessors = vec![id];
        new.authored_at = old.authored_at.clone().or(Some(old.timestamp.clone()));
        new.timestamp = now();
        new.signature = None;
        if self.actor.name != old.author.name {
            new.committer = Some(self.actor.clone());
        }
        self.write_commit(new)
    }

    /// A merge moved elsewhere: what it added on top of merging its parents
    /// goes on top of merging the new ones.
    fn replay_merge(
        &mut self,
        old: &Commit,
        parents: &[ObjectId],
    ) -> Result<(ObjectId, Tree, Vec<Conflict>)> {
        let (base, _) = self.parents_tree(&old.parents)?;
        let (ours, mut conflicts) = self.parents_tree(parents)?;
        let theirs = self.tree(&old.tree)?;
        let merged = merge::merge_trees(self.store, &base, &ours, &theirs)?;
        conflicts.extend(merged.conflicts);
        let tree = self.store.write_tree(&merged.tree)?;
        Ok((tree, merged.tree, conflicts))
    }

    /// Moves `commits` (parents first) onto `dest`. Parents inside the set
    /// follow their new versions; the others are replaced by `dest`.
    /// Bookmarks and workspaces on them move along.
    pub fn rebase(
        &mut self,
        commits: &[ObjectId],
        dest: ObjectId,
    ) -> Result<HashMap<ObjectId, ObjectId>> {
        if commits.contains(&dest) {
            return Err(Error::InvalidArgument(
                "cannot rebase a commit onto itself or its own descendants".to_owned(),
            ));
        }
        let inside: HashSet<ObjectId> = commits.iter().copied().collect();
        let mut moved: HashMap<ObjectId, ObjectId> = HashMap::new();
        for id in commits {
            if *id == self.root_commit {
                return Err(Error::InvalidArgument(
                    "the root commit cannot be rebased".to_owned(),
                ));
            }
            let old = self.commit(id)?;
            let mut parents: Vec<ObjectId> = old
                .parents
                .iter()
                .filter(|p| inside.contains(p))
                .map(|p| moved.get(p).copied().unwrap_or(*p))
                .collect();
            if parents.is_empty() {
                parents.push(dest);
            }
            let new = self.move_commit(*id, parents)?;
            moved.insert(*id, new);
            self.replace_refs(*id, new);
        }
        Ok(moved)
    }

    /// Replays `steps` in order on top of `onto`, for interactive rebases.
    /// Returns the last commit.
    pub fn apply_steps(&mut self, steps: &[Step], onto: ObjectId) -> Result<ObjectId> {
        let mut tip = onto;
        let mut moved: Vec<ObjectId> = Vec::new();
        for step in steps {
            match step {
                Step::Pick(id) => {
                    tip = self.move_commit(*id, vec![tip])?;
                    self.replace_refs(*id, tip);
                    moved.push(tip);
                }
                Step::Reword(id, message) => {
                    let new = self.move_commit(*id, vec![tip])?;
                    let mut commit = self.commit(&new)?;
                    commit.description.clone_from(message);
                    commit.predecessors = vec![*id];
                    commit.signature = None;
                    tip = self.write_commit(commit)?;
                    self.replace_refs(*id, tip);
                    moved.push(tip);
                }
                Step::Squash(id) | Step::Fixup(id) => {
                    if tip == onto {
                        return Err(Error::InvalidArgument(
                            "there is no commit before the first one to fold into".to_owned(),
                        ));
                    }
                    let (tree, _, conflicts) = self.replayed_tree(*id, &[tip], None)?;
                    let folded = self.commit(id)?;
                    let mut commit = self.commit(&tip)?;
                    if matches!(step, Step::Squash(_)) {
                        commit.description =
                            merged_description(&commit.description, &folded.description);
                    }
                    commit.tree = tree;
                    commit.conflicts = conflicts;
                    commit.predecessors.push(*id);
                    commit.signature = None;
                    let new = self.write_commit(commit)?;
                    self.replace_refs(*id, new);
                    self.replace_refs(tip, new);
                    tip = new;
                }
                Step::Drop(id) => self.replace_refs(*id, tip),
            }
        }
        Ok(tip)
    }

    /// Removes a commit from history: its children go on top of its parents
    /// and whatever pointed at it moves to its first parent. Returns where.
    pub fn abandon(&mut self, id: ObjectId) -> Result<ObjectId> {
        if id == self.root_commit {
            return Err(Error::InvalidArgument(
                "the root commit cannot be abandoned".to_owned(),
            ));
        }
        let commit = self.commit(&id)?;
        let parent = commit.parents.first().copied().unwrap_or(self.root_commit);
        for child in self.children_of(id)? {
            let child_commit = self.commit(&child)?;
            let mut parents = Vec::new();
            for p in &child_commit.parents {
                if *p == id {
                    parents.extend(commit.parents.iter().copied());
                } else {
                    parents.push(*p);
                }
            }
            let new = self.move_tree(child, parents)?;
            self.replace_refs(child, new);
        }
        self.view.heads.remove(&id);
        let wc_on_it: Vec<String> = self
            .view
            .workspaces
            .iter()
            .filter(|(_, w)| w.wc == id)
            .map(|(name, _)| name.clone())
            .collect();
        for name in wc_on_it {
            let tree = self.commit(&parent)?.tree;
            let fresh = self.new_commit(vec![parent], tree, String::new(), Vec::new())?;
            if let Some(ws) = self.view.workspaces.get_mut(&name) {
                ws.wc = fresh;
            }
        }
        for target in self.view.bookmarks.values_mut() {
            if *target == id {
                *target = parent;
            }
        }
        Ok(parent)
    }

    /// Moves `id` onto `parents`, and everything that descends from it along.
    pub(crate) fn move_tree(&mut self, id: ObjectId, parents: Vec<ObjectId>) -> Result<ObjectId> {
        let below = self.descendants(id)?;
        let new = self.move_commit(id, parents)?;
        let mut moved = HashMap::from([(id, new)]);
        for child in below {
            let old = self.commit(&child)?;
            let parents = old
                .parents
                .iter()
                .map(|p| moved.get(p).copied().unwrap_or(*p))
                .collect();
            let next = self.move_commit(child, parents)?;
            moved.insert(child, next);
            self.replace_refs(child, next);
        }
        Ok(new)
    }

    /// Writes a new version of `id` and carries its descendants over, so
    /// their contents follow (unlike [`Tx::rewrite`], which keeps their
    /// snapshots: right for descriptions, wrong for files).
    pub fn rewrite_contents(
        &mut self,
        id: ObjectId,
        mutate: impl FnOnce(&mut Commit),
    ) -> Result<ObjectId> {
        let below = self.descendants(id)?;
        let mut commit = self.commit(&id)?;
        mutate(&mut commit);
        commit.predecessors = vec![id];
        commit.timestamp = now();
        commit.signature = None;
        let new = self.write_commit(commit)?;
        self.replace_refs(id, new);
        let mut moved = HashMap::from([(id, new)]);
        for child in below {
            let old = self.commit(&child)?;
            let parents = old
                .parents
                .iter()
                .map(|p| moved.get(p).copied().unwrap_or(*p))
                .collect();
            let next = self.move_commit(child, parents)?;
            moved.insert(child, next);
            self.replace_refs(child, next);
        }
        Ok(new)
    }

    /// Moves changes from `from` into `into`, keeping only `spec`. Commits
    /// left with nothing are dropped, unless they are a working copy.
    pub fn squash(
        &mut self,
        from: ObjectId,
        into: ObjectId,
        spec: &PathSpec,
        message: Option<String>,
    ) -> Result<ObjectId> {
        if from == into {
            return Err(Error::InvalidArgument(
                "cannot squash a change into itself".to_owned(),
            ));
        }
        let source = self.commit(&from)?;
        let (base, _) = self.parents_tree(&source.parents)?;
        let theirs = self.tree(&source.tree)?;
        let target = self.commit_tree(&into)?;
        let moved_side = take_paths(&base, &theirs, spec);
        let merged = merge::merge_trees(self.store, &base, &target, &moved_side)?;
        let tree = self.store.write_tree(&merged.tree)?;
        let description = message.unwrap_or_else(|| {
            let into_text = self.commits_description(into);
            if spec.is_all() {
                merged_description(&into_text, &source.description)
            } else {
                into_text
            }
        });
        let new = self.rewrite_contents(into, |c| {
            c.tree = tree;
            c.conflicts.extend(merged.conflicts);
            c.description = description;
        })?;
        // The source may have moved with the rewrite: find its latest version.
        let latest = self.latest_version(from, source.change_id)?;
        if latest != new
            && self.is_empty_change_ignoring_description(&latest)?
            && !self.view.workspaces.values().any(|w| w.wc == latest)
        {
            self.abandon(latest)?;
        } else if latest != new && self.is_empty_change_ignoring_description(&latest)? {
            // A working copy stays, empty and ready for more work.
            self.rewrite(latest, |c| c.description.clear())?;
        }
        Ok(new)
    }

    fn commits_description(&mut self, id: ObjectId) -> String {
        self.commit(&id).map(|c| c.description).unwrap_or_default()
    }

    /// The visible commit of a change, starting from a version of it.
    fn latest_version(&mut self, id: ObjectId, change: ChangeId) -> Result<ObjectId> {
        if self.tips().contains(&id) {
            return Ok(id);
        }
        for candidate in self.visible()? {
            if self.commit(&candidate)?.change_id == change {
                return Ok(candidate);
            }
        }
        Ok(id)
    }

    fn is_empty_change_ignoring_description(&mut self, id: &ObjectId) -> Result<bool> {
        let commit = self.commit(id)?;
        if !commit.conflicts.is_empty() {
            return Ok(false);
        }
        let (base, _) = self.parents_tree(&commit.parents)?;
        Ok(self.tree(&commit.tree)? == base)
    }

    /// Splits `id` in two: a new change with `spec`, and `id` on top of it
    /// with the rest. Returns the new change.
    pub fn split(&mut self, id: ObjectId, spec: &PathSpec, message: String) -> Result<ObjectId> {
        let commit = self.commit(&id)?;
        let (base, _) = self.parents_tree(&commit.parents)?;
        let full = self.tree(&commit.tree)?;
        let first = take_paths(&base, &full, spec);
        self.split_tree(id, &first, message)
    }

    /// Splits `id` in two: a new change with exactly `first`, and `id` on
    /// top of it. Returns the new change.
    pub fn split_tree(&mut self, id: ObjectId, first: &Tree, message: String) -> Result<ObjectId> {
        let commit = self.commit(&id)?;
        let tree = self.store.write_tree(first)?;
        let mut split = Commit::new(
            ChangeId::random(),
            commit.parents.clone(),
            tree,
            message,
            commit.author.clone(),
            now(),
        );
        split.provenance = commit.provenance.clone();
        split.authored_at = commit.authored_at.clone();
        let new = self.write_commit(split)?;
        self.rewrite(id, |c| c.parents = vec![new])?;
        Ok(new)
    }

    /// Replaces the working-copy commit's files with `tree`.
    pub fn set_wc_tree(&mut self, tree: &Tree, conflicts: Vec<Conflict>) -> Result<ObjectId> {
        let wc = self.wc_id()?;
        let id = self.store.write_tree(tree)?;
        self.rewrite(wc, |c| {
            c.tree = id;
            c.conflicts = conflicts;
        })
    }

    /// Puts the working copy's changes away and leaves it empty. Returns the
    /// stash commit, or `None` if there was nothing to put away.
    pub fn stash_push(
        &mut self,
        message: Option<String>,
        spec: &PathSpec,
    ) -> Result<Option<ObjectId>> {
        let wc = self.wc_id()?;
        let commit = self.commit(&wc)?;
        let (base, _) = self.parents_tree(&commit.parents)?;
        let current = self.tree(&commit.tree)?;
        let stashed = take_paths(&base, &current, spec);
        if stashed == base {
            return Ok(None);
        }
        let tree = self.store.write_tree(&stashed)?;
        let description = message.unwrap_or_else(|| {
            let on = commit
                .parents
                .first()
                .map_or_else(String::new, ObjectId::short);
            format!("WIP on {on}")
        });
        let stash = self.new_commit(commit.parents.clone(), tree, description, Vec::new())?;
        self.view.stashes.insert(0, stash);
        let remaining = take_paths(&current, &base, spec);
        self.set_wc_tree(&remaining, Vec::new())?;
        Ok(Some(stash))
    }

    /// Applies a stash onto the working copy; conflicts are recorded.
    pub fn stash_apply(&mut self, stash: ObjectId) -> Result<Vec<Conflict>> {
        let commit = self.commit(&stash)?;
        let (base, _) = self.parents_tree(&commit.parents)?;
        let theirs = self.tree(&commit.tree)?;
        let wc = self.wc_id()?;
        let ours = self.commit_tree(&wc)?;
        let merged = merge::merge_trees(self.store, &base, &ours, &theirs)?;
        let conflicts = merged.conflicts.clone();
        self.set_wc_tree(&merged.tree, merged.conflicts)?;
        Ok(conflicts)
    }

    /// Git's `reset`: moves the workspace's bookmark to `target` and the
    /// working copy on top of it. With `hard` the files follow; otherwise
    /// they stay, and show up as changes.
    pub fn reset(&mut self, target: ObjectId, hard: bool) -> Result<ObjectId> {
        let wc = self.wc_id()?;
        if let Some(bookmark) = self.current_bookmark() {
            self.view.bookmarks.insert(bookmark, target);
        }
        if hard {
            let tree = self.commit(&target)?.tree;
            let fresh = self.new_commit(vec![target], tree, String::new(), Vec::new())?;
            self.view.heads.remove(&wc);
            if let Some(ws) = self.view.workspaces.get_mut(self.ws_name) {
                ws.wc = fresh;
            }
            return Ok(fresh);
        }
        self.rewrite(wc, |c| c.parents = vec![target])
    }

    /// The bookmark this workspace is on, if it still exists.
    #[must_use]
    pub fn current_bookmark(&self) -> Option<String> {
        let name = self.view.workspaces.get(self.ws_name)?.bookmark.clone()?;
        self.view.bookmarks.contains_key(&name).then_some(name)
    }

    pub fn set_current_bookmark(&mut self, name: Option<String>) {
        if let Some(ws) = self.view.workspaces.get_mut(self.ws_name) {
            ws.bookmark = name;
        }
    }

    /// Bookmarks that contain `id`.
    pub fn bookmarks_containing(&mut self, id: ObjectId) -> Result<BTreeSet<String>> {
        let marks: Vec<(String, ObjectId)> = self
            .view
            .bookmarks
            .iter()
            .map(|(n, i)| (n.clone(), *i))
            .collect();
        let mut out = BTreeSet::new();
        for (name, target) in marks {
            if self.is_ancestor(id, target)? {
                out.insert(name);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_specs_match_files_directories_and_globs() {
        let spec = PathSpec::new(&["src".to_owned(), "docs/*.md".to_owned()]).unwrap();
        assert!(spec.matches("src/a/b.rs"));
        assert!(spec.matches("src"));
        assert!(spec.matches("docs/guide.md"));
        assert!(!spec.matches("lib/src/x.rs"));
        assert!(!spec.matches("docs/img/a.png"));
        assert!(PathSpec::default().matches("anything"));
        assert!(PathSpec::new(&[".".to_owned()]).unwrap().matches("a/b"));
    }
}
