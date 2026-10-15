//! The working copy: turning a directory into a [`Tree`] and back.

use std::collections::BTreeMap;
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
    for entry in walker {
        let entry = entry?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        // Symlinks and special files are not tracked yet.
        if !file_type.is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
        let Some(path) = to_tree_path(rel) else {
            continue;
        };
        let meta = entry.metadata()?;
        let size = meta.len();
        let mtime = mtime_ns(&meta);
        let executable = is_executable(&meta);

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
        tree.files.insert(path, FileEntry { blob, executable });
    }
    cache.entries = fresh;
    Ok(tree)
}

/// Updates the directory from `from` to `to`, touching only files that
/// differ. Untracked and ignored files are left alone.
pub fn checkout(root: &Path, store: &Store, from: &Tree, to: &Tree) -> Result<CheckoutStats> {
    let mut stats = CheckoutStats::default();
    for path in from.files.keys() {
        if !to.files.contains_key(path) {
            let full = safe_join(root, path)?;
            match fs::remove_file(&full) {
                Ok(()) => stats.removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).at(&full),
            }
            prune_empty_dirs(root, &full);
        }
    }
    for (path, entry) in &to.files {
        if from.files.get(path) == Some(entry) {
            continue;
        }
        let full = safe_join(root, path)?;
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        let data = store.read(BLOB, &entry.blob)?;
        fs::write(&full, data).at(&full)?;
        set_executable(&full, entry.executable)?;
        stats.written += 1;
    }
    Ok(stats)
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
}
