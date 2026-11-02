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
//! Concurrency follows Jujutsu. A command reads the latest view without
//! taking any lock, does all its work (merges, imports, checkouts) and then
//! publishes the result in one short transaction; if another process published
//! meanwhile, the two views are merged (see [`oplog::merge_views`]). Agents
//! never wait for each other's work, only for that last instant.
//!
//! Two commands in the same workspace do wait for each other, because they
//! share a directory: a file lock (`.hebra/lock`), which the operating system
//! releases if the process dies.
//!
//! Publishing comes first and the files follow. If a command dies in between,
//! or another workspace rewrites this one's change, the directory is simply
//! behind the view: the next command notices (`WcCache::synced`) and brings it
//! up to date, keeping any edits made meanwhile.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::claims::{self, Claim};
use crate::diff::FileStatus;
use crate::error::{Error, IoContext, Result};
use crate::ids::{ChangeId, ObjectId, is_change_id_prefix};
use crate::merge::{self, has_conflict_markers};
use crate::model::{Actor, Commit, Conflict, ConflictKind, Provenance, Tree, View, WorkspaceInfo};
use crate::oplog::{self, NewOp, OpKind, Operation};
use crate::store::{BLOB, COMMIT, Store, TREE};
use crate::wc::{self, CheckoutStats, DOT_DIR, WcCache};

pub const DEFAULT_WORKSPACE: &str = "default";
/// Other workspaces always looked at before a coordination warning...
const OTHERS_ALWAYS: usize = 8;
/// ...and how long to keep looking at the rest. With hundreds of agents no
/// command can afford to read everyone's directory; starting at a random
/// place means all of them get seen over a few commands.
const OTHERS_BUDGET: Duration = Duration::from_millis(50);
/// Index entries written in the same transaction as the operation; more than
/// this go first, in pieces, so they never hold others back.
const INDEX_INLINE: usize = 64;
const INDEX_CHUNK: usize = 2000;

pub(crate) fn now() -> String {
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
        authored_at: None,
        committer: None,
        signature: None,
        tag: None,
    }
}

pub struct Workspace {
    pub root: PathBuf,
    pub name: String,
    pub repo_dir: PathBuf,
    pub store: Store,
    conn: Connection,
    /// What the last command did to the files of this workspace.
    pub last_checkout: Option<CheckoutStats>,
}

fn open_db(repo_dir: &Path) -> Result<Connection> {
    oplog::open(&repo_dir.join("hebra.db"))
}

fn write_marker(dir: &Path, name: &str, contents: &str) -> Result<()> {
    let path = dir.join(name);
    fs::write(&path, contents).at(path)
}

/// Exclusive use of a workspace's directory, until dropped. The operating
/// system releases it if the process dies, so it can never go stale.
#[derive(Debug)]
pub struct WorkspaceLock {
    _file: File,
}

fn lock_file(root: &Path) -> Result<File> {
    let path = root.join(DOT_DIR).join("lock");
    File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .at(path)
}

fn lock_workspace(root: &Path) -> Result<WorkspaceLock> {
    let file = lock_file(root)?;
    file.lock().at(root.join(DOT_DIR).join("lock"))?;
    Ok(WorkspaceLock { _file: file })
}

fn try_lock_workspace(root: &Path) -> Option<WorkspaceLock> {
    let file = lock_file(root).ok()?;
    file.try_lock().ok()?;
    Some(WorkspaceLock { _file: file })
}

/// How the working directory compares with the working-copy commit.
struct Synced {
    /// The tree to record, when it differs from the commit's.
    tree: Option<(ObjectId, Tree)>,
    cache: WcCache,
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
        let mut wc = Commit::new(
            ChangeId::random(),
            vec![root_id],
            empty_tree,
            String::new(),
            actor.clone(),
            now(),
        );
        wc.provenance = provenance.clone();
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
                bookmark: None,
            },
        );
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('root_commit', ?1)",
            [root_id.hex()],
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
            None,
        )?;
        conn.execute_batch("COMMIT")?;
        Ok(Self {
            root,
            name: DEFAULT_WORKSPACE.to_owned(),
            repo_dir,
            store,
            conn,
            last_checkout: None,
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
            last_checkout: None,
        })
    }

    /// Packs every object still reachable from any recorded operation (so
    /// everything stays undoable) into one file, storing versions of the
    /// same path as deltas, and drops unreachable objects written before
    /// `prune_before`. Operations older than `expire_ops` are forgotten
    /// first, like Git's reflog expiry.
    pub fn gc(
        &mut self,
        prune_before: std::time::SystemTime,
        expire_ops: Option<&str>,
    ) -> Result<crate::store::RepackStats> {
        /// Recent operations always stay undoable.
        const KEEP_OPERATIONS: i64 = 256;
        // Only one at a time; the system frees this if the process dies.
        let lock_path = self.repo_dir.join("gc.lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .at(&lock_path)?;
        if lock.try_lock().is_err() {
            return Err(Error::InvalidArgument(
                "another `hb gc` is running".to_owned(),
            ));
        }
        if let Some(before) = expire_ops {
            oplog::expire(&mut self.conn, before, KEEP_OPERATIONS)?;
        }
        let mut pending: Vec<ObjectId> = oplog::all_tips(&self.conn)?.into_iter().collect();
        let mut keep = HashSet::new();
        let mut objects = Vec::new();
        let mut families: HashMap<String, Vec<ObjectId>> = HashMap::new();
        let mut order = Vec::new();
        while let Some(id) = pending.pop() {
            if !keep.insert(id) {
                continue;
            }
            // Commits from elsewhere may lack their earlier versions.
            let Ok(commit) = self.store.read_json::<Commit>(COMMIT, &id) else {
                continue;
            };
            self.store
                .tree_objects(&commit.tree, &mut keep, &mut objects)?;
            order.push((commit.timestamp.clone(), id));
            pending.extend(commit.parents);
            pending.extend(commit.predecessors);
        }
        // Newest first, so the latest version of each file is the whole one
        // and older versions are the deltas: recent history reads fastest.
        order.sort_unstable_by(|a, b| b.cmp(a));
        for (_, id) in &order {
            let commit: Commit = self.store.read_json(COMMIT, id)?;
            let base = match commit.parents.first() {
                Some(parent) => self
                    .store
                    .read_json::<Commit>(COMMIT, parent)
                    .ok()
                    .map(|p| p.tree),
                None => None,
            };
            self.store
                .walk_changes(base.as_ref(), &commit.tree, &mut |path, object| {
                    families.entry(path).or_default().push(object);
                })?;
        }
        let mut families: Vec<Vec<ObjectId>> = families.into_values().collect();
        // Largest first, so workers stay busy on long histories.
        families.sort_unstable_by_key(|f| std::cmp::Reverse(f.len()));
        let stats = self.store.repack(&keep, &families, prune_before)?;
        oplog::checkpoint(&self.conn)?;
        Ok(stats)
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

    /// The latest view.
    pub fn view(&self) -> Result<View> {
        Ok(oplog::head(&self.conn)?.view)
    }

    fn root_commit_id(&self) -> Result<ObjectId> {
        let hex: String = self
            .conn
            .prepare_cached("SELECT value FROM meta WHERE key = 'root_commit'")?
            .query_row([], |r| r.get(0))?;
        ObjectId::parse(&hex)
    }

    /// Records the working directory into the working-copy commit if it
    /// changed. Runs automatically before every command.
    pub fn snapshot(&mut self, actor: &Actor, provenance: &Provenance) -> Result<bool> {
        let lock = lock_workspace(&self.root)?;
        let changed = self.snapshot_locked(actor, provenance);
        drop(lock);
        changed
    }

    fn snapshot_locked(&mut self, actor: &Actor, provenance: &Provenance) -> Result<bool> {
        let (name, root) = (self.name.clone(), self.root.clone());
        self.snapshot_workspace(&name, &root, actor, provenance)
    }

    /// Snapshots other workspaces whose directory is reachable, so
    /// coordination commands see what agents are touching right now. With
    /// `everyone` false it stops after a short while; workspaces in use are
    /// skipped either way, since waiting for them helps nobody.
    pub fn snapshot_others(&mut self, provenance: &Provenance, everyone: bool) -> Result<()> {
        let view = oplog::head(&self.conn)?.view;
        let mut others: Vec<(String, WorkspaceInfo)> = view
            .workspaces
            .into_iter()
            .filter(|(name, _)| *name != self.name)
            .collect();
        if !everyone && others.len() > OTHERS_ALWAYS {
            let start = rand::random_range(0..others.len());
            others.rotate_left(start);
        }
        let started = Instant::now();
        for (seen, (name, info)) in others.into_iter().enumerate() {
            if !everyone && seen >= OTHERS_ALWAYS && started.elapsed() > OTHERS_BUDGET {
                break;
            }
            let root = PathBuf::from(&info.path);
            let marker = root.join(DOT_DIR).join("workspace");
            if !fs::read_to_string(&marker).is_ok_and(|n| n.trim() == name) {
                continue;
            }
            let Some(lock) = try_lock_workspace(&root) else {
                continue;
            };
            let actor = info
                .actor
                .clone()
                .unwrap_or_else(|| Actor::human("unknown"));
            // Someone else's directory may be changing under us: a failure
            // here must not fail this command.
            let _ = self.snapshot_workspace(&name, &root, &actor, provenance);
            drop(lock);
        }
        Ok(())
    }

    /// Compares a directory with its working-copy commit, without any
    /// repository lock. A directory that fell behind the view is brought up
    /// to date first, keeping edits made meanwhile.
    fn compare_directory(&self, name: &str, root: &Path) -> Result<Option<Synced>> {
        let mut cache = WcCache::load(root);
        let tree = self
            .store
            .batch(|| wc::snapshot(root, &self.store, &mut cache))?;
        let view = oplog::head(&self.conn)?.view;
        let info = view
            .workspaces
            .get(name)
            .ok_or_else(|| Error::WorkspaceNotFound(name.to_owned()))?;
        let current: Commit = self.store.read_json(COMMIT, &info.wc)?;
        let tree = if cache.sparse().is_empty() {
            tree
        } else {
            wc::fill_sparse(root, tree, &self.store.read_tree(&current.tree)?)?
        };
        let disk = match cache.unchanged_tree() {
            Some(id) => id,
            None => self.store.write_tree(&tree)?,
        };
        cache.set_disk_tree(disk);

        if let Some(synced) = cache.synced().filter(|s| *s != current.tree) {
            let base = self.store.read_tree(&synced)?;
            let target = self.store.read_tree(&current.tree)?;
            if disk == synced {
                wc::checkout(root, &self.store, &base, &target)?;
                wc::mark_synced(root, current.tree)?;
                return Ok(None);
            }
            let merged = merge::merge_trees(&self.store, &base, &target, &tree)?;
            wc::checkout(root, &self.store, &tree, &merged.tree)?;
            let id = self.store.write_tree(&merged.tree)?;
            let mut cache = WcCache::load(root);
            cache.set_disk_tree(id);
            return Ok(Some(Synced {
                tree: Some((id, merged.tree)),
                cache,
            }));
        }
        if self.store.same_tree(&disk, &current.tree)? {
            let conflicts = still_conflicted(&self.store, &current.conflicts, &tree)?;
            if conflicts == current.conflicts {
                cache.set_synced(current.tree);
                return Ok(Some(Synced { tree: None, cache }));
            }
        }
        Ok(Some(Synced {
            tree: Some((disk, tree)),
            cache,
        }))
    }

    fn snapshot_workspace(
        &mut self,
        name: &str,
        root: &Path,
        actor: &Actor,
        provenance: &Provenance,
    ) -> Result<bool> {
        let Some(Synced { tree, mut cache }) = self.compare_directory(name, root)? else {
            return Ok(false);
        };
        let Some((tree_id, tree)) = tree else {
            cache.save(root)?;
            return Ok(false);
        };
        let changed = self.transact_locked(name, root, actor, provenance, |tx| {
            let wc_id = tx.wc_id()?;
            let current = tx.commit(&wc_id)?;
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
        })?;
        cache.set_synced(tree_id);
        cache.save(root)?;
        Ok(changed)
    }

    /// Takes the workspace's directory for a whole command: snapshots, runs
    /// `f` and updates the files. Nothing else in this workspace can run
    /// meanwhile; other workspaces are not affected at all.
    ///
    /// `f` works on the view as it was when it started, without any lock, and
    /// its result is published at the end as one operation named `command`.
    /// It must not touch this workspace's files: they follow the view.
    pub fn run<R>(
        &mut self,
        actor: &Actor,
        provenance: &Provenance,
        command: &str,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let lock = lock_workspace(&self.root)?;
        self.snapshot_locked(actor, provenance)?;
        let result = self.transact(actor, provenance, command, OpKind::Command, f)?;
        self.update_directory()?;
        drop(lock);
        Ok(result)
    }

    /// Snapshots, then runs `f` against the latest view without publishing
    /// anything, for commands that only look. Any number of these can run at
    /// once, and they never make anyone wait.
    pub fn read<R>(
        &mut self,
        actor: &Actor,
        provenance: &Provenance,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        self.snapshot(actor, provenance)?;
        self.look(actor, provenance, f)
    }

    /// Like [`Self::read`], without looking at the working directory.
    pub fn look<R>(
        &mut self,
        actor: &Actor,
        provenance: &Provenance,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let root_commit = self.root_commit_id()?;
        let base = oplog::head(&self.conn)?;
        let (name, root) = (self.name.clone(), self.root.clone());
        let mut tx = Tx {
            db: &self.conn,
            store: &self.store,
            ws_name: &name,
            ws_root: &root,
            repo_dir: &self.repo_dir,
            actor,
            provenance,
            root_commit,
            view: base.view,
            commits: HashMap::new(),
            nodes: HashMap::new(),
            new_nodes: Vec::new(),
            op_kind: OpKind::Command,
            undoes: None,
        };
        let result = f(&mut tx)?;
        // Worth keeping only if it spares the next command real work.
        if tx.new_nodes.len() >= INDEX_INLINE {
            let _ = tx.flush_index();
        }
        Ok(result)
    }

    /// Records the directory in one short transaction. Unlike commands this
    /// holds the write lock while `f` runs, so `f` must be small: a snapshot
    /// is the one thing every command does, and it must see the latest view
    /// to avoid racing with a rewrite of its own change.
    fn transact_locked<R>(
        &mut self,
        ws_name: &str,
        ws_root: &Path,
        actor: &Actor,
        provenance: &Provenance,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let root_commit = self.root_commit_id()?;
        let (store, repo_dir) = (&self.store, &self.repo_dir);
        let conn = &self.conn;
        let txn = oplog::retry_busy(|| {
            Ok(Transaction::new_unchecked(
                conn,
                TransactionBehavior::Immediate,
            )?)
        })?;
        let head = oplog::head(&txn)?;
        if !head.view.workspaces.contains_key(ws_name) {
            return Err(Error::WorkspaceNotFound(ws_name.to_owned()));
        }
        let mut tx = Tx {
            db: &txn,
            store,
            ws_name,
            ws_root,
            repo_dir,
            actor,
            provenance,
            root_commit,
            view: head.view.clone(),
            commits: HashMap::new(),
            nodes: HashMap::new(),
            new_nodes: Vec::new(),
            op_kind: OpKind::Snapshot,
            undoes: None,
        };
        let result = f(&mut tx)?;
        let (view, nodes) = (tx.view, tx.nodes);
        if view != head.view {
            for (id, node) in nodes {
                index_insert(&txn, &id, &node)?;
            }
            oplog::insert(
                &txn,
                &NewOp {
                    workspace: ws_name,
                    actor,
                    command: "snapshot",
                    kind: OpKind::Snapshot,
                    undoes: None,
                    view: &view,
                },
                Some(&head.view),
            )?;
        }
        txn.commit()?;
        Ok(result)
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
        let root_commit = self.root_commit_id()?;
        let base = oplog::head(&self.conn)?;
        if !base.view.workspaces.contains_key(&name) {
            return Err(Error::WorkspaceNotFound(name));
        }
        let mut tx = Tx {
            db: &self.conn,
            store: &self.store,
            ws_name: &name,
            ws_root: &root,
            repo_dir: &self.repo_dir,
            actor,
            provenance,
            root_commit,
            view: base.view.clone(),
            commits: HashMap::new(),
            nodes: HashMap::new(),
            new_nodes: Vec::new(),
            op_kind: kind,
            undoes: None,
        };
        let result = f(&mut tx)?;
        if tx.new_nodes.len() > INDEX_INLINE {
            tx.flush_index()?;
        }
        let inline: Vec<(ObjectId, Node)> = tx
            .new_nodes
            .iter()
            .filter_map(|id| tx.nodes.get(id).map(|n| (*id, n.clone())))
            .collect();
        let (view, op_kind, undoes) = (tx.view, tx.op_kind, tx.undoes);
        if view == base.view {
            return Ok(result);
        }
        let conn = &mut self.conn;
        oplog::retry_busy(|| {
            let txn = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let latest = oplog::head(&txn)?;
            let merged;
            let (published, previous) = if latest.id == base.id {
                (&view, &base.view)
            } else {
                merged = oplog::merge_views(&base.view, &view, &latest.view);
                (&merged, &latest.view)
            };
            for (id, node) in &inline {
                index_insert(&txn, id, node)?;
            }
            oplog::insert(
                &txn,
                &NewOp {
                    workspace: &name,
                    actor,
                    command,
                    kind: op_kind,
                    undoes,
                    view: published,
                },
                Some(previous),
            )?;
            txn.commit()?;
            Ok(())
        })?;
        Ok(result)
    }

    /// Makes the files match the working-copy commit of the latest view.
    fn update_directory(&mut self) -> Result<()> {
        self.last_checkout = None;
        let view = oplog::head(&self.conn)?.view;
        let Some(info) = view.workspaces.get(&self.name) else {
            return Ok(());
        };
        let target = self.store.read_json::<Commit>(COMMIT, &info.wc)?.tree;
        let cache = WcCache::load(&self.root);
        let Some(synced) = cache.synced() else {
            wc::mark_synced(&self.root, target)?;
            return Ok(());
        };
        if synced == target {
            return Ok(());
        }
        let from = self.store.read_tree(&synced)?;
        let to = self.store.read_tree(&target)?;
        self.last_checkout = Some(wc::checkout(&self.root, &self.store, &from, &to)?);
        wc::mark_synced(&self.root, target)
    }

    pub fn operations(&self, limit: usize) -> Result<Vec<Operation>> {
        oplog::list(&self.conn, limit)
    }

    pub fn last_activity(&self) -> Result<Vec<(String, String)>> {
        oplog::last_activity(&self.conn)
    }
}

/// What `hb fsck` found.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct FsckReport {
    pub commits: usize,
    pub trees: usize,
    pub blobs: usize,
    /// Objects that are missing or do not match their id.
    pub problems: Vec<String>,
}

impl Workspace {
    /// Reads and verifies every object reachable from every recorded
    /// operation, and the database itself.
    pub fn fsck(&self) -> Result<FsckReport> {
        let mut report = FsckReport::default();
        let integrity: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            report.problems.push(format!("database: {integrity}"));
        }
        let mut pending: Vec<ObjectId> = oplog::all_tips(&self.conn)?.into_iter().collect();
        let mut seen = HashSet::new();
        let mut objects = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let commit: Commit = match self.store.read_json(COMMIT, &id) {
                Ok(commit) => commit,
                Err(e) => {
                    report.problems.push(format!("commit {}: {e}", id.short()));
                    continue;
                }
            };
            report.commits += 1;
            pending.extend(commit.parents);
            if let Err(e) = self
                .store
                .tree_objects(&commit.tree, &mut seen, &mut objects)
            {
                report.problems.push(format!("tree of {}: {e}", id.short()));
            }
        }
        for id in objects {
            // In a partial clone, contents arrive when something reads them.
            if self.store.is_partial() && !self.store.has(&id) {
                continue;
            }
            match self.store.read_raw(&id) {
                Ok(raw) if raw.starts_with(b"tree\0") => report.trees += 1,
                Ok(_) => report.blobs += 1,
                Err(e) => report.problems.push(format!("object {}: {e}", id.short())),
            }
        }
        Ok(report)
    }

    /// An operation and what it changed.
    pub fn operation_delta(&self, id: i64) -> Result<(Operation, oplog::ViewDelta)> {
        let op = oplog::get(&self.conn, id)?;
        let before = match op.parent {
            Some(parent) => oplog::get(&self.conn, parent)?.view,
            None => View::default(),
        };
        let delta = oplog::ViewDelta::between(&before, &op.view);
        Ok((op, delta))
    }

    /// Files and bytes in the object store.
    pub fn store_stats(&self) -> Result<crate::store::StoreStats> {
        self.store.stats()
    }

    /// The tree the working directory is showing, from what the cache
    /// remembers. Sparse workspaces use it to know what is not on disk.
    pub fn sparse_paths(&self) -> Vec<String> {
        WcCache::load(&self.root).sparse().to_vec()
    }

    /// Sets the paths this workspace keeps on disk (none means all) and
    /// updates the files.
    pub fn set_sparse(&mut self, paths: Vec<String>) -> Result<()> {
        let lock = lock_workspace(&self.root)?;
        let view = oplog::head(&self.conn)?.view;
        let info = view
            .workspaces
            .get(&self.name)
            .ok_or_else(|| Error::WorkspaceNotFound(self.name.clone()))?;
        let commit: Commit = self.store.read_json(COMMIT, &info.wc)?;
        let tree = self.store.read_tree(&commit.tree)?;
        wc::set_sparse(&self.root, &self.store, &tree, paths)?;
        drop(lock);
        Ok(())
    }
}

fn index_insert(conn: &Connection, id: &ObjectId, node: &Node) -> Result<()> {
    let mut parents = Vec::with_capacity(node.parents.len() * 32);
    for parent in &node.parents {
        parents.extend_from_slice(parent.as_bytes());
    }
    conn.prepare_cached(
        "INSERT OR IGNORE INTO commit_index (id, generation, time, parents) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![
        id.as_bytes().as_slice(),
        i64::try_from(node.generation).unwrap_or(i64::MAX),
        node.time,
        parents,
    ])?;
    Ok(())
}

fn microseconds(timestamp: &str) -> i64 {
    timestamp
        .parse::<jiff::Timestamp>()
        .map_or(0, jiff::Timestamp::as_microsecond)
}

/// The current time, as commits record it.
#[must_use]
pub fn now_timestamp() -> String {
    now()
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

impl<'a> Tx<'a> {
    /// The database, for what lives outside the view.
    #[must_use]
    pub fn db(&self) -> &'a Connection {
        self.db
    }
}

/// The Git commit a Hebra commit corresponds to.
pub fn git_sha(conn: &Connection, id: &ObjectId) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT git FROM git_map WHERE remote = 'git' AND hebra = ?1")?
        .query_row([id.as_bytes().as_slice()], |r| r.get(0))
        .optional()?)
}

/// The Hebra commit of a Git commit.
pub fn git_commit(conn: &Connection, sha: &str) -> Option<ObjectId> {
    let bytes: Vec<u8> = conn
        .prepare_cached("SELECT hebra FROM git_map WHERE remote = 'git' AND git = ?1")
        .ok()?
        .query_row([sha], |r| r.get(0))
        .ok()?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .ok()
        .map(ObjectId::from_bytes)
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
    /// Graph facts already looked up, and those not yet in the index.
    nodes: HashMap<ObjectId, Node>,
    new_nodes: Vec<ObjectId>,
    op_kind: OpKind,
    undoes: Option<i64>,
}

/// What graph walks need to know about a commit. Kept in the `commit_index`
/// table (it can always be rebuilt from the commits), so walking history does
/// not read one object per commit, like Git's commit-graph.
#[derive(Debug, Clone)]
pub struct Node {
    /// Longest path to the root: always greater than every parent's.
    pub generation: u64,
    /// Microseconds since the epoch.
    pub time: i64,
    pub parents: Vec<ObjectId>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UndoResult {
    pub undone: i64,
    pub command: String,
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
        let mut commit = Commit::new(
            ChangeId::random(),
            parents,
            tree,
            description,
            self.actor.clone(),
            now(),
        );
        commit.provenance = self.provenance.clone();
        commit.conflicts = conflicts;
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
        // A signature covers one exact commit: a new version has none.
        commit.signature = None;
        mutate(&mut commit);
        commit.predecessors = vec![old];
        commit.timestamp = now();
        let new = self.write_commit(commit)?;
        if new == old {
            return Ok(old);
        }
        self.replace_refs(old, new);
        let mut children = self.children_of(old)?;
        children.retain(|id| *id != new);
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

    pub fn replace_refs(&mut self, old: ObjectId, new: ObjectId) {
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
        for tag in self.view.tags.values_mut() {
            if tag.target == old {
                tag.target = new;
            }
        }
    }

    /// What the graph needs to know about a commit: from memory, the index,
    /// or, the first time it is seen, by reading it and its ancestors.
    pub fn node(&mut self, id: &ObjectId) -> Result<Node> {
        if let Some(node) = self.nodes.get(id) {
            return Ok(node.clone());
        }
        if let Some(node) = self.indexed(id)? {
            self.nodes.insert(*id, node.clone());
            return Ok(node);
        }
        // Iterative: histories are far deeper than the stack.
        let mut pending = vec![*id];
        while let Some(&top) = pending.last() {
            if self.nodes.contains_key(&top) {
                pending.pop();
                continue;
            }
            if let Some(node) = self.indexed(&top)? {
                self.nodes.insert(top, node);
                pending.pop();
                continue;
            }
            let commit = self.commit(&top)?;
            let missing: Vec<ObjectId> = commit
                .parents
                .iter()
                .filter(|p| !self.nodes.contains_key(p))
                .copied()
                .collect();
            if missing.is_empty() {
                let generation = commit
                    .parents
                    .iter()
                    .filter_map(|p| self.nodes.get(p))
                    .map(|n| n.generation + 1)
                    .max()
                    .unwrap_or(0);
                let node = Node {
                    generation,
                    time: microseconds(&commit.timestamp),
                    parents: commit.parents,
                };
                self.nodes.insert(top, node);
                self.new_nodes.push(top);
                pending.pop();
            } else {
                pending.extend(missing);
            }
        }
        self.nodes
            .get(id)
            .cloned()
            .ok_or_else(|| Error::RevisionNotFound(id.hex()))
    }

    fn indexed(&self, id: &ObjectId) -> Result<Option<Node>> {
        let row: Option<(i64, i64, Vec<u8>)> = self
            .db
            .prepare_cached("SELECT generation, time, parents FROM commit_index WHERE id = ?1")?
            .query_row([id.as_bytes().as_slice()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((generation, time, parents)) = row else {
            return Ok(None);
        };
        let parents = parents
            .chunks_exact(32)
            .filter_map(|c| <[u8; 32]>::try_from(c).ok())
            .map(ObjectId::from_bytes)
            .collect();
        Ok(Some(Node {
            generation: u64::try_from(generation).unwrap_or(0),
            time,
            parents,
        }))
    }

    /// Saves index entries found so far, a piece at a time.
    fn flush_index(&mut self) -> Result<()> {
        for chunk in self.new_nodes.chunks(INDEX_CHUNK) {
            oplog::retry_busy(|| {
                self.db.execute_batch("BEGIN IMMEDIATE")?;
                for id in chunk {
                    if let Some(node) = self.nodes.get(id)
                        && let Err(e) = index_insert(self.db, id, node)
                    {
                        let _ = self.db.execute_batch("ROLLBACK");
                        return Err(e);
                    }
                }
                self.db.execute_batch("COMMIT")?;
                Ok(())
            })?;
        }
        self.new_nodes.clear();
        Ok(())
    }

    /// Every commit this view keeps visible directly.
    #[must_use]
    pub fn tips(&self) -> Vec<ObjectId> {
        self.view.tips().collect()
    }

    /// Up to `limit` visible commits, newest first, children always before
    /// parents. Stops as soon as it has them: showing the last 20 commits of
    /// a huge history costs the same as of a small one.
    pub fn walk_from(&mut self, tips: &[ObjectId], limit: usize) -> Result<Vec<ObjectId>> {
        let mut ready: BinaryHeap<(u64, i64, ObjectId)> = BinaryHeap::new();
        let mut seen = HashSet::new();
        for id in tips {
            if seen.insert(*id) {
                let node = self.node(id)?;
                ready.push((node.generation, node.time, *id));
            }
        }
        let mut order = Vec::new();
        while order.len() < limit {
            let Some((_, _, id)) = ready.pop() else {
                break;
            };
            order.push(id);
            for parent in self.node(&id)?.parents {
                if seen.insert(parent) {
                    let node = self.node(&parent)?;
                    ready.push((node.generation, node.time, parent));
                }
            }
        }
        Ok(order)
    }

    /// All visible commits, newest first, children always before parents.
    pub fn visible(&mut self) -> Result<Vec<ObjectId>> {
        let tips = self.tips();
        self.walk_from(&tips, usize::MAX)
    }

    /// Visible commits with `id` as a parent. Only looks at commits newer in
    /// the graph than `id`, which is almost nothing for a recent change.
    pub fn children_of(&mut self, id: ObjectId) -> Result<Vec<ObjectId>> {
        let generation = self.node(&id)?.generation;
        let mut pending = self.tips();
        let mut seen = HashSet::new();
        let mut children = Vec::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current) {
                continue;
            }
            let node = self.node(&current)?;
            if node.generation <= generation {
                continue;
            }
            if node.parents.contains(&id) {
                children.push(current);
            }
            pending.extend(node.parents);
        }
        Ok(children)
    }

    /// `start` and all its ancestors.
    pub fn ancestors(&mut self, start: ObjectId) -> Result<HashSet<ObjectId>> {
        let mut seen = HashSet::new();
        let mut pending = vec![start];
        while let Some(id) = pending.pop() {
            if seen.insert(id) {
                pending.extend(self.node(&id)?.parents);
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
                if i != j && self.is_ancestor(*p, *q)? {
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

    /// Whether `ancestor` is `descendant` or one of its ancestors. Does not
    /// look below the generation of `ancestor`.
    pub fn is_ancestor(&mut self, ancestor: ObjectId, descendant: ObjectId) -> Result<bool> {
        if ancestor == descendant {
            return Ok(true);
        }
        let floor = self.node(&ancestor)?.generation;
        let mut pending = vec![descendant];
        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if id == ancestor {
                return Ok(true);
            }
            if seen.insert(id) {
                let node = self.node(&id)?;
                if node.generation > floor {
                    pending.extend(node.parents);
                }
            }
        }
        Ok(false)
    }

    /// The best common ancestors of two commits: none is an ancestor of
    /// another. Usually one; two for a criss-cross merge. Walks newest first
    /// from both sides, like Git's `merge-base`, and stops as soon as only
    /// already-explained commits are left.
    pub fn merge_bases(&mut self, a: ObjectId, b: ObjectId) -> Result<Vec<ObjectId>> {
        const LEFT: u8 = 1;
        const RIGHT: u8 = 2;
        const STALE: u8 = 4;
        const RESULT: u8 = 8;
        const BOTH: u8 = LEFT | RIGHT;
        if a == b {
            return Ok(vec![a]);
        }
        let mut flags: HashMap<ObjectId, u8> = HashMap::new();
        let mut queue: BinaryHeap<(u64, ObjectId)> = BinaryHeap::new();
        flags.insert(a, LEFT);
        flags.insert(b, RIGHT);
        queue.push((self.node(&a)?.generation, a));
        queue.push((self.node(&b)?.generation, b));
        let mut found = Vec::new();
        while queue
            .iter()
            .any(|(_, id)| flags.get(id).is_some_and(|f| f & STALE == 0))
        {
            let Some((_, id)) = queue.pop() else { break };
            let current = flags.get(&id).copied().unwrap_or(0);
            let mut passed = current & (BOTH | STALE);
            if passed == BOTH {
                if current & RESULT == 0 {
                    flags.insert(id, current | RESULT);
                    found.push(id);
                }
                // Everything below a common ancestor is explained by it.
                passed |= STALE;
            }
            for parent in self.node(&id)?.parents {
                let seen = flags.get(&parent).copied().unwrap_or(0);
                if seen & passed != passed {
                    flags.insert(parent, seen | passed);
                    queue.push((self.node(&parent)?.generation, parent));
                }
            }
        }
        found.retain(|id| flags.get(id).is_some_and(|f| f & STALE == 0));
        // A base found early can still be an ancestor of another.
        let mut best = Vec::new();
        for (i, id) in found.iter().enumerate() {
            let mut redundant = false;
            for (j, other) in found.iter().enumerate() {
                if i != j && self.is_ancestor(*id, *other)? {
                    redundant = true;
                    break;
                }
            }
            if !redundant {
                best.push(*id);
            }
        }
        Ok(best)
    }

    /// Nearest common ancestor of two commits.
    pub fn common_ancestor(&mut self, a: ObjectId, b: ObjectId) -> Result<ObjectId> {
        Ok(self
            .merge_bases(a, b)?
            .first()
            .copied()
            .unwrap_or(self.root_commit))
    }

    /// The tree to use as base when merging two commits. With several best
    /// common ancestors they are merged first, like Git's recursive strategy,
    /// which avoids false conflicts after criss-cross merges.
    pub fn merge_base_tree(&mut self, a: ObjectId, b: ObjectId) -> Result<Tree> {
        self.base_tree(a, b, 0)
    }

    fn base_tree(&mut self, a: ObjectId, b: ObjectId, depth: usize) -> Result<Tree> {
        const MAX_DEPTH: usize = 4;
        let bases = self.merge_bases(a, b)?;
        let Some((first, rest)) = bases.split_first() else {
            return Ok(Tree::default());
        };
        let mut tree = self.commit_tree(first)?;
        if depth >= MAX_DEPTH {
            return Ok(tree);
        }
        let mut current = *first;
        for other in rest {
            let base = self.base_tree(current, *other, depth + 1)?;
            let theirs = self.commit_tree(other)?;
            tree = merge::merge_trees(self.store, &base, &tree, &theirs)?.tree;
            current = *other;
        }
        Ok(tree)
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
            let base = self.merge_base_tree(*first, *other)?;
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
        if let [parent] = commit.parents[..] {
            let base = self.commit(&parent)?.tree;
            return self.store.same_tree(&base, &commit.tree);
        }
        let (base, _) = self.parents_tree(&commit.parents)?;
        Ok(self.tree(&commit.tree)? == base)
    }

    /// The files a commit changed relative to its parents. Reads only the
    /// directories that differ.
    pub fn own_changes(&mut self, id: &ObjectId) -> Result<Vec<(String, FileStatus)>> {
        let commit = self.commit(id)?;
        if let [parent] = commit.parents[..] {
            let base = self.commit(&parent)?.tree;
            return Ok(self
                .store
                .tree_delta(&base, &commit.tree)?
                .into_iter()
                .map(|(path, old, new)| {
                    let status = match (old, new) {
                        (None, _) => FileStatus::Added,
                        (_, None) => FileStatus::Deleted,
                        _ => FileStatus::Modified,
                    };
                    (path, status)
                })
                .collect());
        }
        let (base, _) = self.parents_tree(&commit.parents)?;
        let tree = self.tree(&commit.tree)?;
        Ok(crate::diff::changed_paths(&base, &tree))
    }

    /// Points this workspace at `new`; its files follow once the operation
    /// is published. The previous working-copy commit stays visible unless
    /// it is empty.
    pub fn switch_wc(&mut self, new: ObjectId) -> Result<()> {
        let old = self.wc_id()?;
        if old == new {
            return Ok(());
        }
        if !self.is_ancestor(old, new)? && !self.is_empty_change(&old)? {
            self.view.heads.insert(old);
        }
        self.view.heads.remove(&new);
        if let Some(ws) = self.view.workspaces.get_mut(self.ws_name) {
            ws.wc = new;
        }
        Ok(())
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
        let mut commit = Commit::new(
            ChangeId::random(),
            vec![base],
            tree_id,
            description,
            actor.clone().unwrap_or_else(|| saved_actor.clone()),
            now(),
        );
        commit.provenance = Provenance {
            task: task.clone(),
            ..self.provenance.clone()
        };
        let wc = self.write_commit(commit)?;

        let dot = path.join(DOT_DIR);
        fs::create_dir_all(&dot).at(&dot)?;
        write_marker(&dot, "workspace", name)?;
        write_marker(&dot, "repo-path", &self.repo_dir.display().to_string())?;
        write_marker(&dot, ".gitignore", "*\n")?;
        wc::checkout(&path, self.store, &Tree::default(), &base_tree)?;
        wc::mark_synced(&path, tree_id)?;

        let info = WorkspaceInfo {
            wc,
            path: path.display().to_string(),
            actor,
            task,
            created_at: now(),
            bookmark: None,
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
        // Git's spelling: the last finished change.
        if spec == "HEAD" {
            return self.resolve("@-");
        }
        // Git's ancestry: `rev~2` follows first parents, `rev^2` is the
        // second parent. Names may contain neither, so this is unambiguous.
        if let Some(at) = spec.rfind(['~', '^']) {
            let (inner, rest) = spec.split_at(at);
            let count = &rest[1..];
            let n = if count.is_empty() {
                Some(1)
            } else {
                count.parse::<usize>().ok()
            };
            if let Some(n) = n
                && !inner.is_empty()
            {
                let mut id = self.resolve(inner)?;
                if rest.starts_with('^') {
                    if n == 0 {
                        return Ok(id);
                    }
                    return self
                        .commit(&id)?
                        .parents
                        .get(n - 1)
                        .copied()
                        .ok_or_else(|| Error::RevisionNotFound(spec.to_owned()));
                }
                for _ in 0..n {
                    id = self
                        .commit(&id)?
                        .parents
                        .first()
                        .copied()
                        .ok_or_else(|| Error::RevisionNotFound(spec.to_owned()))?;
                }
                return Ok(id);
            }
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
        if let Some(tag) = self.view.tags.get(spec) {
            return Ok(tag.target);
        }
        // `origin/main`, as Git people write `main@origin`.
        if let Some((remote, bookmark)) = spec.split_once('/')
            && let Some(id) = self.view.remotes.get(remote).and_then(|b| b.get(bookmark))
        {
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

    /// Moves `child` from `old_parent` to `new_parent`, with its descendants.
    pub fn rebase_child(
        &mut self,
        child: ObjectId,
        old_parent: ObjectId,
        new_parent: ObjectId,
    ) -> Result<ObjectId> {
        let mut parents = self.commit(&child)?.parents;
        for parent in &mut parents {
            if *parent == old_parent {
                *parent = new_parent;
            }
        }
        let new = self.move_tree(child, parents)?;
        self.replace_refs(child, new);
        Ok(new)
    }

    /// A setting kept in the repository.
    pub fn config_value(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row("SELECT value FROM config WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn config_store(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn note_set(&self, id: &ObjectId, body: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO notes (target, ref, body) VALUES (?1, 'commits', ?2)
             ON CONFLICT (target, ref) DO UPDATE SET body = excluded.body",
            params![id.hex(), body],
        )?;
        Ok(())
    }

    pub fn note_get(&self, id: &ObjectId) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT body FROM notes WHERE target = ?1 AND ref = 'commits'",
                [id.hex()],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn note_remove(&self, id: &ObjectId) -> Result<bool> {
        Ok(self.db.execute(
            "DELETE FROM notes WHERE target = ?1 AND ref = 'commits'",
            [id.hex()],
        )? > 0)
    }

    pub fn note_list(&self) -> Result<Vec<(ObjectId, String)>> {
        let mut stmt = self
            .db
            .prepare("SELECT target, body FROM notes WHERE ref = 'commits' ORDER BY target")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (target, body) = row?;
            out.push((ObjectId::parse(&target)?, body));
        }
        Ok(out)
    }

    /// Records which Git commit each Hebra commit corresponds to.
    pub fn git_map_insert(&self, pairs: &[(ObjectId, String)]) -> Result<()> {
        for chunk in pairs.chunks(INDEX_CHUNK) {
            oplog::retry_busy(|| {
                self.db.execute_batch("BEGIN IMMEDIATE")?;
                for (id, sha) in chunk {
                    let done = self
                        .db
                        .prepare_cached(
                            "INSERT OR REPLACE INTO git_map (remote, hebra, git) VALUES ('git', ?1, ?2)",
                        )
                        .and_then(|mut s| s.execute(params![id.as_bytes().as_slice(), sha]));
                    if let Err(e) = done {
                        let _ = self.db.execute_batch("ROLLBACK");
                        return Err(e.into());
                    }
                }
                self.db.execute_batch("COMMIT")?;
                Ok(())
            })?;
        }
        Ok(())
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
        if before.tags != after.tags {
            restored.tags.clone_from(&before.tags);
        }
        if before.stashes != after.stashes {
            restored.stashes.clone_from(&before.stashes);
        }
        for added in after.heads.difference(&before.heads) {
            restored.heads.remove(added);
        }
        for removed in before.heads.difference(&after.heads) {
            restored.heads.insert(*removed);
        }

        self.view = restored;
        self.op_kind = OpKind::Undo;
        self.undoes = Some(target.id);
        Ok(UndoResult {
            undone: target.id,
            command: target.command,
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
                base.signature = None;
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
                moved.signature = None;
                moved
            };
            new_base = self.write_commit(next)?;
        }
        Ok(new_base)
    }

    /// Restores the whole repository view to what it was after `op`.
    pub fn restore(&mut self, op: i64) -> Result<()> {
        let view = oplog::get(self.db, op)?.view;
        if !view.workspaces.contains_key(self.ws_name) {
            return Err(Error::WorkspaceNotFound(self.ws_name.to_owned()));
        }
        self.op_kind = OpKind::Restore;
        self.view = view;
        Ok(())
    }
}
