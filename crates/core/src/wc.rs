//! The working copy: turning a directory into a [`Tree`] and back.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::attributes::Rules;
use crate::error::{Error, IoContext, Result};
use crate::ids::ObjectId;
use crate::model::{FileEntry, Tree};
use crate::store::{BLOB, Store, write_cache};

/// Directory holding Hebra's own data inside every workspace.
pub const DOT_DIR: &str = ".hebra";
/// Extra ignore file, same syntax as `.gitignore`.
pub const IGNORE_FILE: &str = ".hbignore";

/// Files modified this recently are always re-read, because their mtime
/// cannot yet tell apart two writes in quick succession.
const RACY_WINDOW: Duration = Duration::from_secs(2);
/// With fewer files than this, threads cost more than they save.
const PARALLEL_FROM: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct CacheEntry {
    size: u64,
    mtime_ns: u128,
    blob: ObjectId,
    executable: bool,
}

/// Remembers the blob of each file by size and mtime so unchanged files are
/// not re-hashed on every command.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WcCache {
    entries: BTreeMap<String, CacheEntry>,
    /// Symlinks written as plain files holding their target, because this
    /// system could not create the link (Windows without the privilege).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    links: BTreeSet<String>,
    /// Executable files on systems without an executable bit (Windows), so
    /// the mode is kept instead of silently dropped, like Git's
    /// `core.filemode=false`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    executables: BTreeSet<String>,
    /// Git submodules (their directories are not read), with the blob that
    /// holds the commit each one is pinned at.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    gitlinks: BTreeMap<String, ObjectId>,
    /// The tree of the working-copy commit this directory last matched. If
    /// the commit's tree is another one, the view moved without the files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    synced: Option<ObjectId>,
    /// Only these paths are on disk (empty means all): `hb sparse`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sparse: Vec<String>,
    /// The tree id of the directory at the last snapshot, reused when
    /// nothing changed so unchanged trees are not written again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disk: Option<ObjectId>,
    /// Whether the last snapshot found the directory as this cache knew it.
    #[serde(skip)]
    unchanged: bool,
    /// Whether anything worth saving changed since this was loaded.
    #[serde(skip)]
    dirty: bool,
}

impl WcCache {
    fn path(root: &Path) -> PathBuf {
        root.join(DOT_DIR).join("wc-cache.json")
    }

    /// Loads the cache; a missing or unreadable cache is simply empty.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        fs::read(Self::path(root))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let data = serde_json::to_vec(self)?;
        write_cache(&Self::path(root), |f| std::io::Write::write_all(f, &data))
    }

    /// The tree of the directory, if the last snapshot found no change.
    #[must_use]
    pub fn unchanged_tree(&self) -> Option<ObjectId> {
        self.disk.filter(|_| self.unchanged)
    }

    pub fn set_disk_tree(&mut self, tree: ObjectId) {
        self.dirty |= self.disk != Some(tree);
        self.disk = Some(tree);
    }

    #[must_use]
    pub fn sparse(&self) -> &[String] {
        &self.sparse
    }

    #[must_use]
    pub fn synced(&self) -> Option<ObjectId> {
        self.synced
    }

    pub fn set_synced(&mut self, tree: ObjectId) {
        self.dirty |= self.synced != Some(tree);
        self.synced = Some(tree);
    }
}

/// Keeps only `paths` of `tree` on disk (all of it if empty). Files that
/// leave are deleted, so they must have been recorded first.
pub fn set_sparse(root: &Path, store: &Store, tree: &Tree, paths: Vec<String>) -> Result<()> {
    let mut cache = WcCache::load(root);
    let empty = Tree::default();
    let before = filtered(tree, &cache.sparse)?;
    let after = filtered(tree, &paths)?;
    cache.sparse = paths;
    cache.disk = None;
    cache.dirty = true;
    cache.save(root)?;
    checkout_exact(root, store, &before, &after)?;
    let _ = empty;
    Ok(())
}

/// The part of a tree a sparse workspace keeps.
fn filtered(tree: &Tree, sparse: &[String]) -> Result<Tree> {
    if sparse.is_empty() {
        return Ok(tree.clone());
    }
    let spec = crate::history::PathSpec::new(sparse)?;
    let mut out = tree.clone();
    out.files.retain(|path, _| spec.matches(path));
    Ok(out)
}

/// `tree` with what a sparse workspace does not keep taken from `current`.
pub fn fill_sparse(root: &Path, disk: Tree, current: &Tree) -> Result<Tree> {
    let cache = WcCache::load(root);
    if cache.sparse.is_empty() {
        return Ok(disk);
    }
    let spec = crate::history::PathSpec::new(&cache.sparse)?;
    Ok(crate::history::take_paths(current, &disk, &spec))
}

/// Remembers that the directory matches `tree`.
pub fn mark_synced(root: &Path, tree: ObjectId) -> Result<()> {
    let mut cache = WcCache::load(root);
    cache.set_synced(tree);
    cache.dirty = true;
    cache.save(root)
}

/// Whether bytes look binary (contain a NUL byte), like Git's heuristic.
#[must_use]
pub fn is_binary(data: &[u8]) -> bool {
    data.contains(&0)
}

/// Normalizes CRLF to LF in text files, so line endings never differ between
/// Windows, macOS and Linux.
#[must_use]
pub fn normalize(data: Vec<u8>) -> Vec<u8> {
    if is_binary(&data) || !data.windows(2).any(|w| w == b"\r\n") {
        return data;
    }
    let mut out = Vec::with_capacity(data.len());
    let mut iter = data.iter().peekable();
    while let Some(&b) = iter.next() {
        if b == b'\r' && iter.peek() == Some(&&b'\n') {
            continue;
        }
        out.push(b);
    }
    out
}

/// LF to CRLF in text, for files marked `eol=crlf`.
fn to_crlf(data: Vec<u8>) -> Vec<u8> {
    if is_binary(&data) {
        return data;
    }
    let mut out = Vec::with_capacity(data.len() + data.len() / 32);
    let mut previous = 0u8;
    for &byte in &data {
        if byte == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(byte);
        previous = byte;
    }
    out
}

fn mtime_ns(meta: &fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

/// Converts a relative filesystem path into a tree path (`a/b/c.rs`).
fn to_tree_path(rel: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.to_owned()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// Validates a tree path before it touches the filesystem, so a malicious
/// tree can never write outside the workspace or into `.hebra`.
pub fn safe_join(root: &Path, tree_path: &str) -> Result<PathBuf> {
    let invalid = || Error::InvalidPath(tree_path.to_owned());
    if tree_path.is_empty() || tree_path.starts_with('/') || tree_path.contains('\\') {
        return Err(invalid());
    }
    let mut out = root.to_path_buf();
    for part in tree_path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return Err(invalid());
        }
        if part.eq_ignore_ascii_case(DOT_DIR) || part.eq_ignore_ascii_case(".git") {
            return Err(invalid());
        }
        out.push(part);
    }
    Ok(out)
}

/// Reads the workspace directory into a tree, storing new blobs.
pub fn snapshot(root: &Path, store: &Store, cache: &mut WcCache) -> Result<Tree> {
    let started = SystemTime::now();
    let racy_after = started
        .checked_sub(RACY_WINDOW)
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());

    // Submodules, and any other repository nested inside, are not read.
    let skip_root = root.to_path_buf();
    let submodules: BTreeSet<String> = cache.gitlinks.keys().cloned().collect();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .require_git(false)
        .add_custom_ignore_filename(IGNORE_FILE)
        .filter_entry(move |entry| {
            let name = entry.file_name();
            if name == DOT_DIR || name == ".git" {
                return false;
            }
            let path = entry.path();
            if path == skip_root || !entry.file_type().is_some_and(|t| t.is_dir()) {
                return true;
            }
            let rel = path.strip_prefix(&skip_root).ok().and_then(to_tree_path);
            !rel.is_some_and(|r| submodules.contains(&r)) && !path.join(".git").exists()
        })
        .build();

    let mut tree = Tree::default();
    let mut fresh = BTreeMap::new();
    let mut links = BTreeSet::new();
    let mut executables = BTreeSet::new();
    let mut pending = Vec::new();
    for entry in walker {
        let entry = entry?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
        let Some(path) = to_tree_path(rel) else {
            continue;
        };
        if file_type.is_symlink() {
            let target = fs::read_link(entry.path()).at(entry.path())?;
            let target = target.to_string_lossy().replace('\\', "/");
            let blob = store.write(BLOB, target.as_bytes())?;
            tree.files.insert(path, FileEntry::symlink(blob));
            continue;
        }
        // Sockets, devices and other special files are not tracked.
        if !file_type.is_file() {
            continue;
        }
        if cache.links.contains(&path) {
            let data = fs::read(entry.path()).at(entry.path())?;
            let blob = store.write(BLOB, &data)?;
            tree.files.insert(path.clone(), FileEntry::symlink(blob));
            links.insert(path);
            continue;
        }
        let meta = entry.metadata()?;
        let size = meta.len();
        let mtime = mtime_ns(&meta);
        let executable = is_executable(&meta) || (!cfg!(unix) && cache.executables.contains(&path));
        if executable && !cfg!(unix) {
            executables.insert(path.clone());
        }
        let known = cache.entries.get(&path).copied();
        if let Some(c) = known.filter(|c| {
            c.size == size
                && c.mtime_ns == mtime
                && c.executable == executable
                && mtime < racy_after
        }) {
            fresh.insert(path.clone(), c);
            tree.files.insert(path, FileEntry::file(c.blob, executable));
            continue;
        }
        pending.push(Pending {
            full: entry.path().to_path_buf(),
            path,
            size,
            mtime,
            executable,
            known: known.map(|c| c.blob),
        });
    }
    let rules = if pending.is_empty() {
        Rules::default()
    } else {
        Rules::load(root)
    };
    for (pending, blob) in store_files(store, &rules, pending)? {
        fresh.insert(
            pending.path.clone(),
            CacheEntry {
                size: pending.size,
                mtime_ns: pending.mtime,
                blob,
                executable: pending.executable,
            },
        );
        tree.files
            .insert(pending.path, FileEntry::file(blob, pending.executable));
    }
    // A submodule stays as long as its directory does.
    cache.gitlinks.retain(|path, _| root.join(path).is_dir());
    for (path, blob) in &cache.gitlinks {
        tree.files.insert(path.clone(), FileEntry::gitlink(*blob));
    }
    cache.unchanged =
        fresh == cache.entries && links == cache.links && executables == cache.executables;
    cache.dirty |= !cache.unchanged;
    cache.entries = fresh;
    cache.links = links;
    cache.executables = executables;
    Ok(tree)
}

/// A file that has to be read.
struct Pending {
    full: PathBuf,
    path: String,
    size: u64,
    mtime: u128,
    executable: bool,
    /// Its blob the last time it was seen.
    known: Option<ObjectId>,
}

/// The blob of a file. Line endings are normalized, unless the file is
/// tracked exactly like this (a CRLF file that came from Git must not show
/// up as modified just because it was checked out) or `.gitattributes` says
/// to leave it alone.
fn store_file(store: &Store, rules: &Rules, file: &Pending) -> Result<ObjectId> {
    let data = fs::read(&file.full).at(&file.full)?;
    if let Some(known) = file.known
        && ObjectId::hash(BLOB, &data) == known
    {
        return Ok(known);
    }
    if rules.of(&file.path).text == Some(false) {
        return store.write(BLOB, &data);
    }
    store.write(BLOB, &normalize(data))
}

/// Reads and stores files, on several threads when there are many.
fn store_files(
    store: &Store,
    rules: &Rules,
    files: Vec<Pending>,
) -> Result<Vec<(Pending, ObjectId)>> {
    let workers = workers_for(files.len());
    if workers <= 1 {
        return files
            .into_iter()
            .map(|file| {
                let blob = store_file(store, rules, &file)?;
                Ok((file, blob))
            })
            .collect();
    }
    let next = AtomicUsize::new(0);
    let files = &files;
    let results: Vec<Result<Vec<(usize, ObjectId)>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(file) = files.get(i) else {
                            return Ok(out);
                        };
                        out.push((i, store_file(store, rules, file)?));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::InvalidArgument("a worker panicked".to_owned())))
            })
            .collect()
    });
    let mut blobs = BTreeMap::new();
    for result in results {
        blobs.extend(result?);
    }
    Ok(files
        .iter()
        .enumerate()
        .filter_map(|(i, file)| {
            blobs.get(&i).map(|blob| {
                (
                    Pending {
                        full: file.full.clone(),
                        path: file.path.clone(),
                        size: file.size,
                        mtime: file.mtime,
                        executable: file.executable,
                        known: file.known,
                    },
                    *blob,
                )
            })
        })
        .collect())
}

/// Threads worth using for this many files.
fn workers_for(files: usize) -> usize {
    if files < PARALLEL_FROM {
        return 1;
    }
    std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
}

/// Registers a submodule at `path`, pinned at the commit stored in `blob`.
pub fn add_gitlink(root: &Path, path: &str, blob: ObjectId) -> Result<()> {
    let mut cache = WcCache::load(root);
    cache.gitlinks.insert(path.to_owned(), blob);
    cache.disk = None;
    cache.dirty = true;
    cache.save(root)
}

/// Ignored files in the directory, for `hb clean -x`.
pub fn ignored_files(root: &Path) -> Result<Vec<String>> {
    let tracked: BTreeSet<PathBuf> = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .require_git(false)
        .add_custom_ignore_filename(IGNORE_FILE)
        .build()
        .filter_map(std::result::Result::ok)
        .map(ignore::DirEntry::into_path)
        .collect();
    let mut out = Vec::new();
    let all = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .filter_entry(|e| e.file_name() != DOT_DIR && e.file_name() != ".git")
        .build();
    for entry in all.filter_map(std::result::Result::ok) {
        if entry.file_type().is_some_and(|t| t.is_file()) && !tracked.contains(entry.path()) {
            let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
            if let Some(path) = to_tree_path(rel) {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Updates the directory from `from` to `to`, keeping to the paths of a
/// sparse workspace.
pub fn checkout(root: &Path, store: &Store, from: &Tree, to: &Tree) -> Result<CheckoutStats> {
    let sparse = WcCache::load(root).sparse;
    if sparse.is_empty() {
        return checkout_exact(root, store, from, to);
    }
    checkout_exact(
        root,
        store,
        &filtered(from, &sparse)?,
        &filtered(to, &sparse)?,
    )
}

/// Updates the directory from `from` to `to`, touching only files that
/// differ. Untracked and ignored files are left alone. Nothing is ever
/// written through a symlink, so a tree cannot reach outside the workspace.
fn checkout_exact(root: &Path, store: &Store, from: &Tree, to: &Tree) -> Result<CheckoutStats> {
    let mut stats = CheckoutStats::default();
    for (path, old) in &from.files {
        let replaced = to
            .files
            .get(path)
            .is_none_or(|new| new.gitlink != old.gitlink);
        if !replaced {
            continue;
        }
        let full = safe_join(root, path)?;
        if inside_symlink(root, &full) {
            continue;
        }
        if old.gitlink {
            // Like Git: a submodule's own checkout is never deleted, only
            // its empty directory.
            if fs::remove_dir(&full).is_ok() {
                stats.removed += 1;
            }
        } else if remove_entry(&full)? {
            stats.removed += 1;
        }
        prune_empty_dirs(root, &full);
    }
    // Modes this system cannot hold on disk, remembered in the cache.
    let mut kept_links = Vec::new();
    let mut kept_executables = Vec::new();
    let mut kept_gitlinks = Vec::new();
    let mut files = Vec::new();
    let mut dirs = BTreeSet::new();
    let mut safe_dirs = std::collections::HashSet::new();
    for (path, entry) in &to.files {
        if from.files.get(path) == Some(entry) {
            continue;
        }
        let full = safe_join(root, path)?;
        if inside_symlink_cached(root, &full, &mut safe_dirs) {
            return Err(Error::InvalidPath(path.clone()));
        }
        if let Some(parent) = full.parent() {
            dirs.insert(parent.to_path_buf());
        }
        if entry.gitlink {
            kept_gitlinks.push((path.clone(), entry.blob, full));
            continue;
        }
        if entry.symlink {
            kept_links.push((path.clone(), entry.blob, full));
            continue;
        }
        if entry.executable && !cfg!(unix) {
            kept_executables.push(path.clone());
        }
        files.push((path.as_str(), *entry, full));
    }
    for dir in &dirs {
        fs::create_dir_all(dir).at(dir)?;
    }
    // What is being written decides, not what happens to be on disk yet.
    let rules = match to.files.get(".gitattributes") {
        Some(entry) => Rules::parse(&String::from_utf8_lossy(&store.read(BLOB, &entry.blob)?)),
        None => Rules::default(),
    };
    let written = write_files(store, &rules, &files)?;

    stats.written += files.len();

    let mut links_on_disk = Vec::new();
    for (path, blob, full) in kept_links {
        let data = store.read(BLOB, &blob)?;
        // Replace links instead of writing through them.
        if fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
            remove_entry(&full)?;
        }
        let target = String::from_utf8_lossy(&data).into_owned();
        if !make_symlink(&full, &target) {
            fs::write(&full, &data).at(&full)?;
            links_on_disk.push(path);
        }
        stats.written += 1;
    }
    let mut gitlinks = Vec::new();
    for (path, blob, full) in kept_gitlinks {
        // An empty directory, filled by `git submodule update`.
        fs::create_dir_all(&full).at(&full)?;
        gitlinks.push((path, blob));
        stats.written += 1;
    }
    let forgotten: Vec<&String> = from
        .files
        .iter()
        .filter(|(path, e)| {
            (e.symlink || e.executable || e.gitlink) && to.files.get(*path) != Some(*e)
        })
        .map(|(path, _)| path)
        .collect();
    if root.join(DOT_DIR).is_dir() {
        let mut cache = WcCache::load(root);
        for path in forgotten {
            cache.links.remove(path);
            cache.executables.remove(path);
            cache.gitlinks.remove(path);
        }
        for path in from.files.keys() {
            if !to.files.contains_key(path) {
                cache.entries.remove(path);
            }
        }
        cache.links.extend(links_on_disk);
        cache.executables.extend(kept_executables);
        cache.gitlinks.extend(gitlinks);
        // Written just now and known exactly: the next snapshot need not
        // read them (once they are old enough to trust their timestamps).
        cache.entries.extend(written);
        cache.disk = None;
        cache.dirty = true;
        cache.save(root)?;
    }
    Ok(stats)
}

/// Writes regular files, on several threads when there are many, and
/// returns what the cache should know about each.
fn write_files(
    store: &Store,
    rules: &Rules,
    files: &[(&str, FileEntry, PathBuf)],
) -> Result<Vec<(String, CacheEntry)>> {
    if store.is_partial() {
        let blobs: Vec<ObjectId> = files.iter().map(|(_, e, _)| e.blob).collect();
        store.prefetch(&blobs)?;
    }
    let write_one =
        |(path, entry, full): &(&str, FileEntry, PathBuf)| -> Result<(String, CacheEntry)> {
            let data = store.read(BLOB, &entry.blob)?;
            // Replace links instead of writing through them.
            if fs::symlink_metadata(full).is_ok_and(|m| m.file_type().is_symlink()) {
                remove_entry(full)?;
            }
            let data = if rules.of(path).crlf == Some(true) {
                to_crlf(data)
            } else {
                data
            };
            fs::write(full, &data).at(full)?;
            set_executable(full, entry.executable)?;
            let meta = fs::metadata(full).at(full)?;
            Ok((
                (*path).to_owned(),
                CacheEntry {
                    size: meta.len(),
                    mtime_ns: mtime_ns(&meta),
                    blob: entry.blob,
                    executable: entry.executable,
                },
            ))
        };
    let workers = workers_for(files.len());
    if workers <= 1 {
        return files.iter().map(write_one).collect();
    }
    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::with_capacity(files.len()));
    let failure = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(i) else {
                        return;
                    };
                    match write_one(file) {
                        Ok(done) => {
                            if let Ok(mut out) = out.lock() {
                                out.push(done);
                            }
                        }
                        Err(e) => {
                            if let Ok(mut failure) = failure.lock() {
                                *failure = Some(e);
                            }
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().ok().flatten() {
        return Err(error);
    }
    Ok(out.into_inner().unwrap_or_default())
}

/// Whether a directory between `root` and `full` is a symlink.
/// [`inside_symlink`], remembering directories found to be real ones: a
/// checkout writes many files into few directories.
fn inside_symlink_cached(
    root: &Path,
    full: &Path,
    safe: &mut std::collections::HashSet<PathBuf>,
) -> bool {
    let mut dir = full.parent();
    let mut checked = Vec::new();
    while let Some(d) = dir {
        if d == root || safe.contains(d) {
            break;
        }
        if fs::symlink_metadata(d).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
        checked.push(d.to_path_buf());
        dir = d.parent();
    }
    safe.extend(checked);
    false
}

fn inside_symlink(root: &Path, full: &Path) -> bool {
    let mut dir = full.parent();
    while let Some(d) = dir {
        if d == root {
            return false;
        }
        if fs::symlink_metadata(d).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
        dir = d.parent();
    }
    false
}

/// Removes a file or a symlink (to a file or a directory). Returns whether
/// something was there.
fn remove_entry(full: &Path) -> Result<bool> {
    match fs::remove_file(full) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        // Windows removes directory symlinks with `remove_dir`.
        Err(_) if fs::symlink_metadata(full).is_ok_and(|m| m.file_type().is_symlink()) => {
            fs::remove_dir(full).at(full)?;
            Ok(true)
        }
        Err(e) => Err(e).at(full),
    }
}

#[cfg(unix)]
fn make_symlink(full: &Path, target: &str) -> bool {
    std::os::unix::fs::symlink(target, full).is_ok()
}

#[cfg(windows)]
fn make_symlink(full: &Path, target: &str) -> bool {
    let target = target.replace('/', "\\");
    let points_to_dir = full
        .parent()
        .is_some_and(|parent| parent.join(&target).is_dir());
    if points_to_dir {
        std::os::windows::fs::symlink_dir(&target, full).is_ok()
    } else {
        std::os::windows::fs::symlink_file(&target, full).is_ok()
    }
}

#[cfg(not(any(unix, windows)))]
fn make_symlink(_full: &Path, _target: &str) -> bool {
    false
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CheckoutStats {
    pub written: usize,
    pub removed: usize,
}

fn prune_empty_dirs(root: &Path, removed_file: &Path) {
    let mut dir = removed_file.parent();
    while let Some(d) = dir {
        if d == root || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(path).at(path)?;
    let mut perms = meta.permissions();
    let mode = perms.mode();
    let new_mode = if executable {
        mode | 0o111
    } else {
        mode & !0o111
    };
    if new_mode != mode {
        perms.set_mode(new_mode);
        fs::set_permissions(path, perms).at(path)?;
    }
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_crlf_in_text_only() {
        assert_eq!(normalize(b"a\r\nb\r\n".to_vec()), b"a\nb\n");
        assert_eq!(normalize(b"a\r\n\0".to_vec()), b"a\r\n\0");
        assert_eq!(normalize(b"lone\rcr".to_vec()), b"lone\rcr");
    }

    #[test]
    fn rejects_paths_escaping_the_workspace() {
        let root = Path::new("/ws");
        assert!(safe_join(root, "src/main.rs").is_ok());
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            ".hebra/repo",
            "a/.git/config",
            "",
            "a//b",
            "c:x",
        ] {
            assert!(safe_join(root, bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn snapshot_and_checkout_round_trip() {
        let repo = tempfile::tempdir().unwrap();
        let store = Store::new(repo.path().join("objects"));
        let a = tempfile::tempdir().unwrap();
        fs::write(a.path().join("one.txt"), "1\r\n").unwrap();
        fs::create_dir_all(a.path().join("dir")).unwrap();
        fs::write(a.path().join("dir/two.txt"), "2\n").unwrap();
        fs::write(a.path().join(".gitignore"), "ignored.log\n").unwrap();
        fs::write(a.path().join("ignored.log"), "x").unwrap();
        let tree = snapshot(a.path(), &store, &mut WcCache::default()).unwrap();
        assert_eq!(
            tree.files.keys().collect::<Vec<_>>(),
            [".gitignore", "dir/two.txt", "one.txt"]
        );

        let b = tempfile::tempdir().unwrap();
        checkout(b.path(), &store, &Tree::default(), &tree).unwrap();
        assert_eq!(fs::read_to_string(b.path().join("one.txt")).unwrap(), "1\n");
        checkout(b.path(), &store, &tree, &Tree::default()).unwrap();
        assert!(!b.path().join("dir").exists());
    }

    #[test]
    fn symlinks_round_trip_even_without_link_support() {
        let repo = tempfile::tempdir().unwrap();
        let store = Store::new(repo.path().join("objects"));
        let target = store.write(BLOB, b"real.txt").unwrap();
        let real = store.write(BLOB, b"data\n").unwrap();
        let mut tree = Tree::default();
        tree.files
            .insert("real.txt".to_owned(), FileEntry::file(real, false));
        tree.files
            .insert("link.txt".to_owned(), FileEntry::symlink(target));

        let ws = tempfile::tempdir().unwrap();
        fs::create_dir_all(ws.path().join(DOT_DIR)).unwrap();
        checkout(ws.path(), &store, &Tree::default(), &tree).unwrap();
        let mut cache = WcCache::load(ws.path());
        assert_eq!(snapshot(ws.path(), &store, &mut cache).unwrap(), tree);
        cache.save(ws.path()).unwrap();

        // Back to a regular file: the link is replaced, never written through.
        let mut plain = tree.clone();
        plain
            .files
            .insert("link.txt".to_owned(), FileEntry::file(real, false));
        checkout(ws.path(), &store, &tree, &plain).unwrap();
        assert_eq!(fs::read(ws.path().join("real.txt")).unwrap(), b"data\n");
        let mut cache = WcCache::load(ws.path());
        assert_eq!(snapshot(ws.path(), &store, &mut cache).unwrap(), plain);
    }

    #[cfg(unix)]
    #[test]
    fn never_writes_through_a_symlinked_directory() {
        let repo = tempfile::tempdir().unwrap();
        let store = Store::new(repo.path().join("objects"));
        let outside = tempfile::tempdir().unwrap();
        let target = store
            .write(BLOB, outside.path().to_str().unwrap().as_bytes())
            .unwrap();
        let evil = store.write(BLOB, b"pwned").unwrap();
        let mut first = Tree::default();
        first
            .files
            .insert("dir".to_owned(), FileEntry::symlink(target));
        let mut second = first.clone();
        second
            .files
            .insert("dir/file".to_owned(), FileEntry::file(evil, false));

        let ws = tempfile::tempdir().unwrap();
        checkout(ws.path(), &store, &Tree::default(), &first).unwrap();
        assert!(checkout(ws.path(), &store, &first, &second).is_err());
        assert!(!outside.path().join("file").exists());
    }
}
