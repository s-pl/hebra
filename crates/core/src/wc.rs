//! The working copy: turning a directory into a [`Tree`] and back.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::ids::ObjectId;
use crate::model::{FileEntry, Tree};
use crate::store::{BLOB, Store, write_atomic};

/// Directory holding Hebra's own data inside every workspace.
pub const DOT_DIR: &str = ".hebra";
/// Extra ignore file, same syntax as `.gitignore`.
pub const IGNORE_FILE: &str = ".hbignore";

/// Files modified this recently are always re-read, because their mtime
/// cannot yet tell apart two writes in quick succession.
const RACY_WINDOW: Duration = Duration::from_secs(2);

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
        let data = serde_json::to_vec(self)?;
        write_atomic(&Self::path(root), |f| std::io::Write::write_all(f, &data))
    }
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

    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .add_custom_ignore_filename(IGNORE_FILE)
        .filter_entry(|entry| {
            let name = entry.file_name();
            name != DOT_DIR && name != ".git"
        })
        .build();

    let mut tree = Tree::default();
    let mut fresh = BTreeMap::new();
    let mut links = BTreeSet::new();
    let mut executables = BTreeSet::new();
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

        let cached = cache.entries.get(&path).copied().filter(|c| {
            c.size == size
                && c.mtime_ns == mtime
                && c.executable == executable
                && mtime < racy_after
        });
        let blob = if let Some(c) = cached {
            c.blob
        } else {
            let data = fs::read(entry.path()).at(entry.path())?;
            store.write(BLOB, &normalize(data))?
        };
        fresh.insert(
            path.clone(),
            CacheEntry {
                size,
                mtime_ns: mtime,
                blob,
                executable,
            },
        );
        tree.files.insert(path, FileEntry::file(blob, executable));
    }
    cache.entries = fresh;
    cache.links = links;
    cache.executables = executables;
    Ok(tree)
}

/// Updates the directory from `from` to `to`, touching only files that
/// differ. Untracked and ignored files are left alone. Nothing is ever
/// written through a symlink, so a tree cannot reach outside the workspace.
pub fn checkout(root: &Path, store: &Store, from: &Tree, to: &Tree) -> Result<CheckoutStats> {
    let mut stats = CheckoutStats::default();
    for path in from.files.keys() {
        if !to.files.contains_key(path) {
            let full = safe_join(root, path)?;
            if inside_symlink(root, &full) {
                continue;
            }
            if remove_entry(&full)? {
                stats.removed += 1;
            }
            prune_empty_dirs(root, &full);
        }
    }
    // Modes this system cannot hold on disk, remembered in the cache.
    let mut kept_links = Vec::new();
    let mut kept_executables = Vec::new();
    for (path, entry) in &to.files {
        if from.files.get(path) == Some(entry) {
            continue;
        }
        let full = safe_join(root, path)?;
        if inside_symlink(root, &full) {
            return Err(Error::InvalidPath(path.clone()));
        }
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        let data = store.read(BLOB, &entry.blob)?;
        // Replace links instead of writing through them.
        if fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
            remove_entry(&full)?;
        }
        if entry.symlink {
            let target = String::from_utf8_lossy(&data).into_owned();
            if !make_symlink(&full, &target) {
                fs::write(&full, &data).at(&full)?;
                kept_links.push(path.clone());
            }
        } else {
            fs::write(&full, data).at(&full)?;
            set_executable(&full, entry.executable)?;
            if entry.executable && !cfg!(unix) {
                kept_executables.push(path.clone());
            }
        }
        stats.written += 1;
    }
    let forgotten: Vec<&String> = from
        .files
        .iter()
        .filter(|(path, e)| (e.symlink || e.executable) && to.files.get(*path) != Some(*e))
        .map(|(path, _)| path)
        .collect();
    let changed = !(kept_links.is_empty() && kept_executables.is_empty() && forgotten.is_empty());
    if changed && root.join(DOT_DIR).is_dir() {
        let mut cache = WcCache::load(root);
        for path in forgotten {
            cache.links.remove(path);
            cache.executables.remove(path);
        }
        cache.links.extend(kept_links);
        cache.executables.extend(kept_executables);
        cache.save(root)?;
    }
    Ok(stats)
}

/// Whether a directory between `root` and `full` is a symlink.
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
