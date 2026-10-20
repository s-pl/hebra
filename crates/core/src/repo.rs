//! Workspaces, transactions and the commit graph.
//!
//! Layout on disk:
//!
//! ```text
//! <root>/.hebra/workspace        name of this workspace ("default")
//! <root>/.hebra/repo/hebra.db     operation log, config, claims (SQLite)
//! <root>/.hebra/repo/objects/     content-addressed objects
//! <other>/.hebra/workspace       name of another workspace
//! <other>/.hebra/repo-path       absolute path of the shared repo dir
//! ```
//!
//! Every command runs in two SQLite `IMMEDIATE` transactions: first the
//! working directory is snapshotted into the workspace's working-copy commit,
//! then the command itself runs against the latest view. Concurrent agents in
//! different workspaces are serialized by SQLite and never corrupt each other.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::claims::{self, Claim};
use crate::error::{Error, IoContext, Result};
use crate::ids::{ChangeId, ObjectId, is_change_id_prefix};
use crate::merge::{self, has_conflict_markers};
use crate::model::{Actor, Commit, Conflict, ConflictKind, Provenance, Tree, View, WorkspaceInfo};
use crate::oplog::{self, NewOp, OpKind, Operation};
use crate::store::{BLOB, COMMIT, Store, TREE};
use crate::wc::{self, CheckoutStats, DOT_DIR, WcCache};

pub const DEFAULT_WORKSPACE: &str = "default";
const SCHEMA_VERSION: &str = "1";

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn root_commit() -> Commit {
    Commit {
        change_id: ChangeId::root(),
        parents: Vec::new(),
        predecessors: Vec::new(),
        tree: ObjectId::hash(
            TREE,
            &serde_json::to_vec(&Tree::default()).unwrap_or_default(),
        ),
        description: String::new(),
        author: Actor::human("root"),
        timestamp: "1970-01-01T00:00:00Z".to_owned(),
        provenance: Provenance::default(),
        conflicts: Vec::new(),
    }
}

pub struct Workspace {
    pub root: PathBuf,
    pub name: String,
    pub repo_dir: PathBuf,
    pub store: Store,
    conn: Connection,
}

fn open_db(repo_dir: &Path) -> Result<Connection> {
    let conn = Connection::open(repo_dir.join("hebra.db"))?;
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.execute_batch(oplog::SCHEMA)?;
    Ok(conn)
}

fn write_marker(dir: &Path, name: &str, contents: &str) -> Result<()> {
    let path = dir.join(name);
    fs::write(&path, contents).at(path)
}

impl Workspace {
    /// Creates a repository in `root` and imports whatever files are already
    /// there (respecting `.gitignore` and `.hbignore`).
    pub fn init(root: &Path, actor: &Actor, provenance: &Provenance) -> Result<Self> {
        let root = std::path::absolute(root).at(root)?;
        let dot = root.join(DOT_DIR);
        if dot.exists() {
            return Err(Error::AlreadyInitialized(root));
        }
        let repo_dir = dot.join("repo");
        fs::create_dir_all(repo_dir.join("objects")).at(&repo_dir)?;
        write_marker(&dot, "workspace", DEFAULT_WORKSPACE)?;
        write_marker(&dot, ".gitignore", "*\n")?;

        let store = Store::new(repo_dir.join("objects"));
        let conn = open_db(&repo_dir)?;
        let empty_tree = store.write_tree(&Tree::default())?;
        let root_id = store.write_json(COMMIT, &root_commit())?;
        let wc = Commit {
            change_id: ChangeId::random(),
            parents: vec![root_id],
            predecessors: Vec::new(),
            tree: empty_tree,
            description: String::new(),
            author: actor.clone(),
            timestamp: now(),
            provenance: provenance.clone(),
            conflicts: Vec::new(),
        };
        let wc_id = store.write_json(COMMIT, &wc)?;
        let mut view = View::default();
        view.workspaces.insert(
            DEFAULT_WORKSPACE.to_owned(),
            WorkspaceInfo {
                wc: wc_id,
                path: root.display().to_string(),
                actor: None,
                task: None,
                created_at: now(),
            },
        );
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1), ('root_commit', ?2)",
            [SCHEMA_VERSION, &root_id.hex()],
        )?;
        oplog::insert(
            &conn,
            &NewOp {
                workspace: DEFAULT_WORKSPACE,
                actor,
                command: "init",
                kind: OpKind::Init,
                undoes: None,
                view: &view,
            },
        )?;
        conn.execute_batch("COMMIT")?;
        Ok(Self {
            root,
            name: DEFAULT_WORKSPACE.to_owned(),
            repo_dir,
            store,
            conn,
        })
    }

    /// Finds the workspace containing `start` (searching parent directories).
    pub fn open(start: &Path) -> Result<Self> {
        let start = std::path::absolute(start).at(start)?;
        let root = start
            .ancestors()
            .find(|dir| dir.join(DOT_DIR).join("workspace").is_file())
            .ok_or(Error::NotAWorkspace)?
            .to_path_buf();
        let dot = root.join(DOT_DIR);
        let name = fs::read_to_string(dot.join("workspace"))
            .at(dot.join("workspace"))?
            .trim()
            .to_owned();
        let pointer = dot.join("repo-path");
        let repo_dir = if pointer.is_file() {
            PathBuf::from(fs::read_to_string(&pointer).at(&pointer)?.trim())
        } else {
            dot.join("repo")
        };
        if !repo_dir.join("hebra.db").is_file() {
            return Err(Error::NotAWorkspace);
        }
        let store = Store::new(repo_dir.join("objects"));
        let conn = open_db(&repo_dir)?;
        Ok(Self {
            root,
            name,
            repo_dir,
            store,
            conn,
        })
    }

    /// Packs every object still reachable from any recorded operation (so
    /// everything stays undoable) into one file, and drops unreachable
    /// objects written before `prune_before`.
    pub fn gc(&self, prune_before: std::time::SystemTime) -> Result<crate::store::RepackStats> {
        let mut roots: Vec<ObjectId> = Vec::new();
        let mut views = Vec::new();
        {
            let mut stmt = self.conn.prepare("SELECT view FROM operations")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for row in rows {
                views.push(serde_json::from_str::<View>(&row?)?);
            }
        }
        for view in &views {
            roots.extend(view.workspaces.values().map(|w| w.wc));
            roots.extend(view.bookmarks.values());
            roots.extend(view.heads.iter());
            roots.extend(view.remotes.values().flat_map(|b| b.values()));
        }
        let mut keep = HashSet::new();
        let mut objects = Vec::new();
        while let Some(id) = roots.pop() {
            if !keep.insert(id) {
                continue;
            }
            // Commits from elsewhere may lack their earlier versions.
            let Ok(commit) = self.store.read_json::<Commit>(COMMIT, &id) else {
                continue;
            };
            self.store
                .tree_objects(&commit.tree, &mut keep, &mut objects)?;
            roots.extend(commit.parents);
            roots.extend(commit.predecessors);
        }
        self.store.repack(&keep, prune_before)
    }

    pub fn config_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM config WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn config_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn config_unset(&self, key: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM config WHERE key = ?1", [key])?
            > 0)
    }

    pub fn config_list(&self) -> Result<BTreeMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM config")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// This workspace's entry in the latest view (agent identity, task...).
    pub fn info(&self) -> Result<Option<WorkspaceInfo>> {
        Ok(oplog::head(&self.conn)?
            .view
            .workspaces
            .get(&self.name)
            .cloned())
    }

    fn root_commit_id(&self) -> Result<ObjectId> {
        let hex: String = self.conn.query_row(
            "SELECT value FROM meta WHERE key = 'root_commit'",
            [],
            |r| r.get(0),
        )?;
        ObjectId::parse(&hex)
    }

    /// Records the working directory into the working-copy commit if it
    /// changed. Runs automatically before every command.
    pub fn snapshot(&mut self, actor: &Actor, provenance: &Provenance) -> Result<bool> {
        let (name, root) = (self.name.clone(), self.root.clone());
        self.snapshot_workspace(&name, &root, actor, provenance)
    }

    /// Snapshots every other workspace whose directory is reachable, so
    /// coordination commands see what all agents are touching right now.
    pub fn snapshot_others(&mut self, provenance: &Provenance) -> Result<()> {
        let view = oplog::head(&self.conn)?.view;
        for (name, info) in view.workspaces {
            if name == self.name {
                continue;
            }
            let root = PathBuf::from(&info.path);
            let marker = root.join(DOT_DIR).join("workspace");
            let matches = fs::read_to_string(&marker).is_ok_and(|n| n.trim() == name);
            if !matches {
                continue;
            }
            let actor = info
                .actor
                .clone()
                .unwrap_or_else(|| Actor::human("unknown"));
            self.snapshot_workspace(&name, &root, &actor, provenance)?;
        }
        Ok(())
    }

    fn snapshot_workspace(
        &mut self,
        name: &str,
        root: &Path,
        actor: &Actor,
        provenance: &Provenance,
    ) -> Result<bool> {
        let mut cache = WcCache::load(root);
        let tree = self
            .store
            .batch(|| wc::snapshot(root, &self.store, &mut cache))?;
        let changed = self.transact_in(
            name,
            root,
            actor,
            provenance,
            "snapshot",
            OpKind::Snapshot,
            |tx| {
                let wc_id = tx.wc_id()?;
                let current = tx.commit(&wc_id)?;
                let mut tree_id = tx.store.write_tree(&tree)?;
                // Same files under an older tree encoding: keep the old id.
                if tree_id != current.tree && tx.tree(&current.tree)? == tree {
                    tree_id = current.tree;
                }
                let conflicts = still_conflicted(tx.store, &current.conflicts, &tree)?;
                if tree_id == current.tree && conflicts == current.conflicts {
                    return Ok(false);
                }
                // Whoever puts the first content into an empty change is its author.
                let adopt = tx.is_empty_change(&wc_id)?;
                let (actor, provenance) = (tx.actor.clone(), tx.provenance.clone());
                tx.rewrite(wc_id, |c| {
                    c.tree = tree_id;
                    c.conflicts = conflicts;
                    if adopt {
                        c.author = actor;
                        if !provenance.is_empty() {
                            c.provenance = provenance;
                        }
                    }
                })?;
                Ok(true)
            },
        )?;
        cache.save(root)?;
        Ok(changed)
    }

    /// Snapshots, then runs `f` in a transaction. If the view changed, the
    /// result is recorded as one operation named `command`.
    pub fn run<R>(
        &mut self,
        actor: &Actor,
        provenance: &Provenance,
        command: &str,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        self.snapshot(actor, provenance)?;
        self.transact(actor, provenance, command, OpKind::Command, f)
    }

    fn transact<R>(
        &mut self,
        actor: &Actor,
        provenance: &Provenance,
        command: &str,
        kind: OpKind,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let (name, root) = (self.name.clone(), self.root.clone());
        self.transact_in(&name, &root, actor, provenance, command, kind, f)
    }

    #[allow(clippy::too_many_arguments)]
    fn transact_in<R>(
        &mut self,
        ws_name: &str,
        ws_root: &Path,
        actor: &Actor,
        provenance: &Provenance,
        command: &str,
        kind: OpKind,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let root_commit = self.root_commit_id()?;
        let txn = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head = oplog::head(&txn)?;
        if !head.view.workspaces.contains_key(ws_name) {
            return Err(Error::WorkspaceNotFound(ws_name.to_owned()));
        }
        let mut tx = Tx {
            db: &txn,
            store: &self.store,
            ws_name,
            ws_root,
            repo_dir: &self.repo_dir,
            actor,
            provenance,
            root_commit,
            view: head.view.clone(),
            commits: HashMap::new(),
            op_kind: kind,
            undoes: None,
        };
        let result = f(&mut tx)?;
        let (view, op_kind, undoes) = (tx.view, tx.op_kind, tx.undoes);
        if view != head.view {
            oplog::insert(
                &txn,
                &NewOp {
                    workspace: ws_name,
                    actor,
                    command,
                    kind: op_kind,
                    undoes,
                    view: &view,
                },
            )?;
        }
        txn.commit()?;
        Ok(result)
    }

    pub fn operations(&self, limit: usize) -> Result<Vec<Operation>> {
        oplog::list(&self.conn, limit)
    }

    pub fn last_activity(&self) -> Result<Vec<(String, String)>> {
        oplog::last_activity(&self.conn)
    }
}

/// Drops content conflicts whose markers are gone from the file.
fn still_conflicted(store: &Store, conflicts: &[Conflict], tree: &Tree) -> Result<Vec<Conflict>> {
    let mut out = Vec::new();
    for conflict in conflicts {
        let Some(entry) = tree.files.get(&conflict.path) else {
            continue;
        };
        if conflict.kind == ConflictKind::Content
            && !has_conflict_markers(&store.read(BLOB, &entry.blob)?)
        {
            continue;
        }
        out.push(conflict.clone());
    }
    Ok(out)
}

/// A transaction over the repository view.
pub struct Tx<'a> {
    db: &'a Connection,
    pub store: &'a Store,
    pub ws_name: &'a str,
    pub ws_root: &'a Path,
    pub repo_dir: &'a Path,
    pub actor: &'a Actor,
    pub provenance: &'a Provenance,
    pub root_commit: ObjectId,
    pub view: View,
    commits: HashMap<ObjectId, Commit>,
    op_kind: OpKind,
    undoes: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UndoResult {
    pub undone: i64,
    pub command: String,
    pub checkout: Option<CheckoutStats>,
    /// Workspaces and bookmarks with later work, carried over onto where
    /// they were before (`workspace default`, `bookmark main`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rebased: Vec<String>,
    /// Workspaces and bookmarks left as they are, because later work could
    /// not be traced back to the undone operation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<String>,
}

/// What undoing an operation does to one workspace or bookmark.
enum RefUndo {
    /// Nothing happened to it since: back to its earlier value.
    Restore(Option<ObjectId>),
    /// Later work, replayed on top of its earlier value.
    Rebased(ObjectId),
    /// Later work that does not descend from the undone change: untouched.
    Kept,
}

impl Tx<'_> {
    pub fn commit(&mut self, id: &ObjectId) -> Result<Commit> {
        if let Some(c) = self.commits.get(id) {
            return Ok(c.clone());
        }
        let c: Commit = self.store.read_json(COMMIT, id)?;
        self.commits.insert(*id, c.clone());
        Ok(c)
    }

    pub fn tree(&self, id: &ObjectId) -> Result<Tree> {
        self.store.read_tree(id)
    }

    pub fn commit_tree(&mut self, id: &ObjectId) -> Result<Tree> {
        let tree = self.commit(id)?.tree;
        self.tree(&tree)
    }

    pub fn write_commit(&mut self, commit: Commit) -> Result<ObjectId> {
        let id = self.store.write_json(COMMIT, &commit)?;
        self.commits.insert(id, commit);
        Ok(id)
    }

    pub fn wc_id(&self) -> Result<ObjectId> {
        self.wc_of(self.ws_name)
    }

    pub fn wc_of(&self, workspace: &str) -> Result<ObjectId> {
        self.view
            .workspaces
            .get(workspace)
            .map(|w| w.wc)
            .ok_or_else(|| Error::WorkspaceNotFound(workspace.to_owned()))
    }

    /// Writes a brand-new change on top of `parents`.
    pub fn new_commit(
        &mut self,
        parents: Vec<ObjectId>,
        tree: ObjectId,
        description: String,
        conflicts: Vec<Conflict>,
    ) -> Result<ObjectId> {
        let commit = Commit {
            change_id: ChangeId::random(),
            parents,
            predecessors: Vec::new(),
            tree,
            description,
            author: self.actor.clone(),
            timestamp: now(),
            provenance: self.provenance.clone(),
            conflicts,
        };
        self.write_commit(commit)
    }

    /// Writes a new version of `old` (same change id) and updates everything
    /// that pointed at it. Visible descendants are re-parented onto the new
    /// version, keeping their own snapshots.
    pub fn rewrite(&mut self, old: ObjectId, mutate: impl FnOnce(&mut Commit)) -> Result<ObjectId> {
        self.rewrite_dyn(old, Box::new(mutate))
    }

    fn rewrite_dyn(
        &mut self,
        old: ObjectId,
        mutate: Box<dyn FnOnce(&mut Commit) + '_>,
    ) -> Result<ObjectId> {
        let mut commit = self.commit(&old)?;
        mutate(&mut commit);
        commit.predecessors = vec![old];
        commit.timestamp = now();
        let new = self.write_commit(commit)?;
        if new == old {
            return Ok(old);
        }
        self.replace_refs(old, new);
        let children: Vec<ObjectId> = self
            .visible()?
            .into_iter()
            .filter(|id| *id != new)
            .filter(|id| {
                self.commits
                    .get(id)
                    .is_some_and(|c| c.parents.contains(&old))
            })
            .collect();
        for child in children {
            self.rewrite_dyn(
                child,
                Box::new(move |c: &mut Commit| {
                    for parent in &mut c.parents {
                        if *parent == old {
                            *parent = new;
                        }
                    }
                }),
            )?;
        }
        Ok(new)
    }

    fn replace_refs(&mut self, old: ObjectId, new: ObjectId) {
        for ws in self.view.workspaces.values_mut() {
            if ws.wc == old {
                ws.wc = new;
            }
        }
        for target in self.view.bookmarks.values_mut() {
            if *target == old {
                *target = new;
            }
        }
        if self.view.heads.remove(&old) {
            self.view.heads.insert(new);
        }
    }

    /// All visible commits, newest first, children always before parents.
    pub fn visible(&mut self) -> Result<Vec<ObjectId>> {
        let mut tips: Vec<ObjectId> = self.view.workspaces.values().map(|w| w.wc).collect();
        tips.extend(self.view.bookmarks.values().copied());
        tips.extend(self.view.heads.iter().copied());
        tips.extend(self.view.remotes.values().flat_map(|b| b.values().copied()));

        let mut seen = HashSet::new();
        let mut queue: VecDeque<ObjectId> = tips.into_iter().collect();
        while let Some(id) = queue.pop_front() {
            if seen.insert(id) {
                queue.extend(self.commit(&id)?.parents);
            }
        }
        // Kahn's algorithm, newest timestamp first among ready commits.
        let mut pending_children: HashMap<ObjectId, usize> =
            seen.iter().map(|id| (*id, 0)).collect();
        for id in &seen {
            for parent in self.commit(id)?.parents {
                *pending_children.entry(parent).or_default() += 1;
            }
        }
        let mut ready: BinaryHeap<(String, ObjectId)> = BinaryHeap::new();
        for (id, count) in &pending_children {
            if *count == 0 {
                ready.push((self.commit(id)?.timestamp, *id));
            }
        }
        let mut order = Vec::with_capacity(seen.len());
        while let Some((_, id)) = ready.pop() {
            order.push(id);
            for parent in self.commit(&id)?.parents {
                if let Some(count) = pending_children.get_mut(&parent) {
                    *count -= 1;
                    if *count == 0 {
                        ready.push((self.commit(&parent)?.timestamp, parent));
                    }
                }
            }
        }
        Ok(order)
    }

    fn ancestors(&mut self, start: ObjectId) -> Result<HashSet<ObjectId>> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([start]);
        while let Some(id) = queue.pop_front() {
            if seen.insert(id) {
                queue.extend(self.commit(&id)?.parents);
            }
        }
        Ok(seen)
    }

    /// Drops parents that are ancestors of other parents (they add nothing
    /// to a merge), keeping order.
    pub fn simplify_parents(&mut self, parents: Vec<ObjectId>) -> Result<Vec<ObjectId>> {
        let mut unique = Vec::new();
        for p in parents {
            if !unique.contains(&p) {
                unique.push(p);
            }
        }
        let mut keep = Vec::new();
        for (i, p) in unique.iter().enumerate() {
            let mut redundant = false;
            for (j, q) in unique.iter().enumerate() {
                if i != j && self.ancestors(*q)?.contains(p) {
                    redundant = true;
                    break;
                }
            }
            if !redundant {
                keep.push(*p);
            }
        }
        Ok(keep)
    }

    /// Whether `ancestor` is `descendant` or one of its ancestors.
    pub fn is_ancestor(&mut self, ancestor: ObjectId, descendant: ObjectId) -> Result<bool> {
        Ok(self.ancestors(descendant)?.contains(&ancestor))
    }

    /// Nearest common ancestor of two commits (breadth-first from `b`).
    pub fn common_ancestor(&mut self, a: ObjectId, b: ObjectId) -> Result<ObjectId> {
        let of_a = self.ancestors(a)?;
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([b]);
        while let Some(id) = queue.pop_front() {
            if of_a.contains(&id) {
                return Ok(id);
            }
            if seen.insert(id) {
                queue.extend(self.commit(&id)?.parents);
            }
        }
        Ok(self.root_commit)
    }

    /// The tree a change with these parents starts from (a merge of the
    /// parents for merge commits), plus any conflicts that merge produced.
    pub fn parents_tree(&mut self, parents: &[ObjectId]) -> Result<(Tree, Vec<Conflict>)> {
        let Some((first, rest)) = parents.split_first() else {
            return Ok((Tree::default(), Vec::new()));
        };
        let mut tree = self.commit_tree(first)?;
        let mut conflicts = Vec::new();
        for other in rest {
            let base_id = self.common_ancestor(*first, *other)?;
            let base = self.commit_tree(&base_id)?;
            let theirs = self.commit_tree(other)?;
            let result = merge::merge_trees(self.store, &base, &tree, &theirs)?;
            tree = result.tree;
            conflicts.extend(result.conflicts);
        }
        Ok((tree, conflicts))
    }

    /// Whether a commit has no description and no changes of its own.
    pub fn is_empty_change(&mut self, id: &ObjectId) -> Result<bool> {
        let commit = self.commit(id)?;
        if !commit.description.is_empty() || !commit.conflicts.is_empty() {
            return Ok(false);
        }
        let (base, _) = self.parents_tree(&commit.parents)?;
        Ok(self.tree(&commit.tree)? == base)
    }

    /// Points this workspace at `new` and updates its files on disk.
    /// The previous working-copy commit stays visible unless it is empty.
    pub fn switch_wc(&mut self, new: ObjectId) -> Result<CheckoutStats> {
        let old = self.wc_id()?;
        if old == new {
            return Ok(CheckoutStats::default());
        }
        let old_tree = self.commit_tree(&old)?;
        let new_tree = self.commit_tree(&new)?;
        let new_ancestors = self.ancestors(new)?;
        if !new_ancestors.contains(&old) && !self.is_empty_change(&old)? {
            self.view.heads.insert(old);
        }
        self.view.heads.remove(&new);
        if let Some(ws) = self.view.workspaces.get_mut(self.ws_name) {
            ws.wc = new;
        }
        wc::checkout(self.ws_root, self.store, &old_tree, &new_tree)
    }

    /// Creates a new workspace with its own directory and working-copy change.
    pub fn create_workspace(
        &mut self,
        name: &str,
        path: &Path,
        base: ObjectId,
        actor: Option<Actor>,
        task: Option<String>,
    ) -> Result<WorkspaceInfo> {
        if self.view.workspaces.contains_key(name) {
            return Err(Error::WorkspaceExists(name.to_owned()));
        }
        if path.exists() && fs::read_dir(path).at(path)?.next().is_some() {
            return Err(Error::InvalidArgument(format!(
                "{} exists and is not empty",
                path.display()
            )));
        }
        let path = std::path::absolute(path).at(path)?;
        let base_tree = self.commit_tree(&base)?;
        let tree_id = self.commit(&base)?.tree;
        let description = task.clone().unwrap_or_default();
        let saved_actor = self.actor;
        let commit = Commit {
            change_id: ChangeId::random(),
            parents: vec![base],
            predecessors: Vec::new(),
            tree: tree_id,
            description,
            author: actor.clone().unwrap_or_else(|| saved_actor.clone()),
            timestamp: now(),
            provenance: Provenance {
                task: task.clone(),
                ..self.provenance.clone()
            },
            conflicts: Vec::new(),
        };
        let wc = self.write_commit(commit)?;

        let dot = path.join(DOT_DIR);
        fs::create_dir_all(&dot).at(&dot)?;
        write_marker(&dot, "workspace", name)?;
        write_marker(&dot, "repo-path", &self.repo_dir.display().to_string())?;
        write_marker(&dot, ".gitignore", "*\n")?;
        wc::checkout(&path, self.store, &Tree::default(), &base_tree)?;

        let info = WorkspaceInfo {
            wc,
            path: path.display().to_string(),
            actor,
            task,
            created_at: now(),
        };
        self.view.workspaces.insert(name.to_owned(), info.clone());
        Ok(info)
    }

    /// Resolves a revision: `@`, `<workspace>@`, `<rev>-` (parent),
    /// `root()`, a bookmark, a change id prefix (`k`-`z`) or a commit id
    /// prefix (hex).
    pub fn resolve(&mut self, spec: &str) -> Result<ObjectId> {
        let spec = spec.trim();
        if spec == "@" {
            return self.wc_id();
        }
        if spec == "root()" {
            return Ok(self.root_commit);
        }
        if let Some(inner) = spec.strip_suffix('-') {
            let id = self.resolve(inner)?;
            return self
                .commit(&id)?
                .parents
                .first()
                .copied()
                .ok_or_else(|| Error::RevisionNotFound(spec.to_owned()));
        }
        if let Some(ws) = spec.strip_suffix('@') {
            return self
                .wc_of(ws)
                .map_err(|_| Error::RevisionNotFound(spec.to_owned()));
        }
        if let Some((bookmark, remote)) = spec.rsplit_once('@') {
            return self
                .view
                .remotes
                .get(remote)
                .and_then(|b| b.get(bookmark))
                .copied()
                .ok_or_else(|| Error::RevisionNotFound(spec.to_owned()));
        }
        if let Some(id) = self.view.bookmarks.get(spec) {
            return Ok(*id);
        }
        let visible = self.visible()?;
        if is_change_id_prefix(spec) {
            let mut changes: BTreeMap<ChangeId, ObjectId> = BTreeMap::new();
            for id in &visible {
                let commit = self.commit(id)?;
                if commit.change_id.encode().starts_with(spec) {
                    // `visible` is newest first: keep the newest version.
                    changes.entry(commit.change_id).or_insert(*id);
                }
            }
            return match changes.len() {
                0 => Err(Error::RevisionNotFound(spec.to_owned())),
                1 => Ok(*changes.values().next().unwrap_or(&self.root_commit)),
                _ => Err(Error::AmbiguousRevision(spec.to_owned())),
            };
        }
        if spec.len() >= 4 && spec.bytes().all(|b| b.is_ascii_hexdigit()) {
            let spec = spec.to_ascii_lowercase();
            let matches: BTreeSet<ObjectId> = visible
                .into_iter()
                .filter(|id| id.hex().starts_with(&spec))
                .collect();
            return match matches.len() {
                0 => ObjectId::parse(&spec)
                    .ok()
                    .filter(|id| self.commit(id).is_ok())
                    .ok_or_else(|| Error::RevisionNotFound(spec.clone())),
                1 => Ok(*matches.iter().next().unwrap_or(&self.root_commit)),
                _ => Err(Error::AmbiguousRevision(spec)),
            };
        }
        Err(Error::RevisionNotFound(spec.to_owned()))
    }

    pub fn claims(&self) -> Result<Vec<Claim>> {
        claims::active(self.db)
    }

    pub fn claim(&self, claim: &Claim) -> Result<()> {
        claims::upsert(self.db, claim)
    }

    pub fn release(&self, paths: &[String]) -> Result<usize> {
        claims::release(self.db, self.ws_name, paths)
    }

    /// Reverts the most recent operation that is not a snapshot (or already
    /// undone). Only what that operation changed is reverted, so concurrent
    /// work in other workspaces is kept.
    pub fn undo(&mut self) -> Result<UndoResult> {
        let mut cursor = oplog::head(self.db)?;
        let target = loop {
            match cursor.kind {
                OpKind::Snapshot => {}
                OpKind::Undo => {
                    let undone = oplog::get(self.db, cursor.undoes.unwrap_or(cursor.id))?;
                    cursor = undone;
                }
                OpKind::Init => return Err(Error::InvalidArgument("nothing to undo".to_owned())),
                OpKind::Command | OpKind::Restore => break cursor,
            }
            let parent = cursor
                .parent
                .ok_or_else(|| Error::InvalidArgument("nothing to undo".to_owned()))?;
            cursor = oplog::get(self.db, parent)?;
        };
        let before = oplog::get(
            self.db,
            target
                .parent
                .ok_or_else(|| Error::InvalidArgument("nothing to undo".to_owned()))?,
        )?
        .view;
        let after = &target.view;
        let mut restored = self.view.clone();
        let (mut rebased, mut kept) = (Vec::new(), Vec::new());

        let names: BTreeSet<String> = before
            .workspaces
            .keys()
            .chain(after.workspaces.keys())
            .cloned()
            .collect();
        for name in names {
            let (b, a) = (before.workspaces.get(&name), after.workspaces.get(&name));
            if b == a {
                continue;
            }
            let current = self.view.workspaces.get(&name).map(|w| w.wc);
            match self.undo_ref(b.map(|w| w.wc), a.map(|w| w.wc), current)? {
                RefUndo::Restore(_) => match b {
                    Some(info) => restored.workspaces.insert(name.clone(), info.clone()),
                    None => restored.workspaces.remove(&name),
                },
                RefUndo::Rebased(id) => {
                    if let Some(info) = restored.workspaces.get_mut(&name) {
                        info.wc = id;
                    }
                    rebased.push(format!("workspace {name}"));
                    None
                }
                RefUndo::Kept => {
                    kept.push(format!("workspace {name}"));
                    None
                }
            };
        }
        let names: BTreeSet<String> = before
            .bookmarks
            .keys()
            .chain(after.bookmarks.keys())
            .cloned()
            .collect();
        for name in names {
            let (b, a) = (before.bookmarks.get(&name), after.bookmarks.get(&name));
            if b == a {
                continue;
            }
            let current = self.view.bookmarks.get(&name).copied();
            match self.undo_ref(b.copied(), a.copied(), current)? {
                RefUndo::Restore(Some(id)) => {
                    restored.bookmarks.insert(name.clone(), id);
                }
                RefUndo::Restore(None) => {
                    restored.bookmarks.remove(&name);
                }
                RefUndo::Rebased(id) => {
                    restored.bookmarks.insert(name.clone(), id);
                    rebased.push(format!("bookmark {name}"));
                }
                RefUndo::Kept => kept.push(format!("bookmark {name}")),
            }
        }
        if before.remotes != after.remotes {
            restored.remotes.clone_from(&before.remotes);
        }
        for added in after.heads.difference(&before.heads) {
            restored.heads.remove(added);
        }
        for removed in before.heads.difference(&after.heads) {
            restored.heads.insert(*removed);
        }

        let checkout = self.apply_view(restored)?;
        self.op_kind = OpKind::Undo;
        self.undoes = Some(target.id);
        Ok(UndoResult {
            undone: target.id,
            command: target.command,
            checkout,
            rebased,
            kept,
        })
    }

    /// Undoing an operation that moved a reference from `before` to
    /// `after`, now that it points at `current`.
    fn undo_ref(
        &mut self,
        before: Option<ObjectId>,
        after: Option<ObjectId>,
        current: Option<ObjectId>,
    ) -> Result<RefUndo> {
        if current == after {
            return Ok(RefUndo::Restore(before));
        }
        let (Some(b), Some(a), Some(c)) = (before, after, current) else {
            return Ok(RefUndo::Kept);
        };
        match self.later_chain(a, c)? {
            Some(chain) => Ok(RefUndo::Rebased(self.replay(b, a, &chain)?)),
            None => Ok(RefUndo::Kept),
        }
    }

    /// The commits from `after` (excluded, or its rewrite included) up to
    /// `current`, oldest first, following first parents.
    fn later_chain(&mut self, after: ObjectId, current: ObjectId) -> Result<Option<Vec<ObjectId>>> {
        const LIMIT: usize = 10_000;
        let after_change = self.commit(&after)?.change_id;
        let mut chain = Vec::new();
        let mut id = current;
        for _ in 0..LIMIT {
            if id == after {
                chain.reverse();
                return Ok(Some(chain));
            }
            let commit = self.commit(&id)?;
            chain.push(id);
            if commit.change_id == after_change {
                chain.reverse();
                return Ok(Some(chain));
            }
            match commit.parents.first() {
                Some(parent) if *parent != self.root_commit => id = *parent,
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    /// Replays `chain` (work done after `after`) onto `before`. A rewrite of
    /// `after` itself becomes `before` plus what changed since `after`; the
    /// commits on top of it are moved onto the result one by one. Conflicts
    /// are recorded in the commits, as in any merge.
    fn replay(
        &mut self,
        before: ObjectId,
        after: ObjectId,
        chain: &[ObjectId],
    ) -> Result<ObjectId> {
        let after_commit = self.commit(&after)?;
        let mut new_base = before;
        for id in chain {
            let commit = self.commit(id)?;
            let rewrite_of_after = commit.change_id == after_commit.change_id;
            let base_tree = if rewrite_of_after {
                self.tree(&after_commit.tree)?
            } else {
                match commit.parents.first() {
                    Some(parent) => self.commit_tree(parent)?,
                    None => Tree::default(),
                }
            };
            let ours = self.commit_tree(&new_base)?;
            let theirs = self.tree(&commit.tree)?;
            let merged = merge::merge_trees(self.store, &base_tree, &ours, &theirs)?;
            let tree = self.store.write_tree(&merged.tree)?;
            let next = if rewrite_of_after {
                let mut base = self.commit(&new_base)?;
                if commit.description != after_commit.description {
                    base.description.clone_from(&commit.description);
                }
                base.predecessors = vec![new_base, *id];
                base.tree = tree;
                base.conflicts = merged.conflicts;
                base.timestamp = now();
                base
            } else {
                let mut moved = commit.clone();
                if let Some(first) = moved.parents.first_mut() {
                    *first = new_base;
                }
                moved.predecessors = vec![*id];
                moved.tree = tree;
                moved.conflicts = merged.conflicts;
                moved.timestamp = now();
                moved
            };
            new_base = self.write_commit(next)?;
        }
        Ok(new_base)
    }

    /// Restores the whole repository view to what it was after `op`.
    pub fn restore(&mut self, op: i64) -> Result<Option<CheckoutStats>> {
        let view = oplog::get(self.db, op)?.view;
        if !view.workspaces.contains_key(self.ws_name) {
            return Err(Error::WorkspaceNotFound(self.ws_name.to_owned()));
        }
        self.op_kind = OpKind::Restore;
        self.apply_view(view)
    }

    fn apply_view(&mut self, view: View) -> Result<Option<CheckoutStats>> {
        let old_wc = self.wc_id()?;
        let new_wc = view.workspaces.get(self.ws_name).map(|w| w.wc);
        self.view = view;
        match new_wc {
            Some(new_wc) if new_wc != old_wc => {
                let from = self.commit_tree(&old_wc)?;
                let to = self.commit_tree(&new_wc)?;
                Ok(Some(wc::checkout(self.ws_root, self.store, &from, &to)?))
            }
            _ => Ok(None),
        }
    }
}
