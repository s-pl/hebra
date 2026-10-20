//! Content-addressed object store.
//!
//! Each object lives in `objects/<2 hex>/<62 hex>`. Its id is the BLAKE3 hash
//! of `<kind>` + NUL + `<data>`, so every read can be verified.
//!
//! With encryption keys (servers), files hold `HBE1 | key id (8) | nonce (24)
//! | XChaCha20-Poly1305 ciphertext` and the object id is bound to the
//! ciphertext as associated data, so encrypted files cannot be swapped. The
//! first key encrypts; every key can decrypt, which allows key rotation.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, IoContext, Result};
use crate::ids::ObjectId;
use crate::model::{FileEntry, StoredTree, TREE_VERSION, Tree};

pub const BLOB: &str = "blob";
pub const TREE: &str = "tree";
pub const COMMIT: &str = "commit";

const MAGIC: &[u8; 4] = b"HBE1";
/// Deeper trees are rejected, so hostile input cannot exhaust the stack.
const MAX_TREE_DEPTH: usize = 256;
const KEY_ID_LEN: usize = 8;
const NONCE_LEN: usize = 24;

/// A 256-bit encryption key and its public id (derived, safe to store).
#[derive(Clone)]
pub struct Key {
    id: [u8; KEY_ID_LEN],
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key({})", hex::encode(self.id))
    }
}

impl Key {
    /// Parses a key written as 64 hex characters.
    pub fn from_hex(text: &str) -> Result<Self> {
        let bytes = hex::decode(text.trim())
            .map_err(|_| Error::InvalidArgument("encryption key must be hex".to_owned()))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
            Error::InvalidArgument("encryption key must be 32 bytes (64 hex characters)".to_owned())
        })?;
        let digest = blake3::derive_key("hebra object encryption key id v1", &bytes);
        let mut id = [0u8; KEY_ID_LEN];
        id.copy_from_slice(&digest[..KEY_ID_LEN]);
        let key = chacha20poly1305::Key::from(bytes);
        Ok(Self {
            id,
            cipher: XChaCha20Poly1305::new(&key),
        })
    }

    /// A new random key, as hex.
    #[must_use]
    pub fn generate_hex() -> String {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes);
        hex::encode(bytes)
    }
}

/// Where object files live. Keys look like `ab/cdef…` (2 + 62 hex).
///
/// Objects are immutable and content-addressed, so a backend only needs
/// get / put-if-absent / exists; any blob store (local disk, S3) fits.
pub trait Backend: Send + Sync + std::fmt::Debug {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>>;
    fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()>;
    fn exists(&self, key: &str) -> std::io::Result<bool>;
}

/// Objects as files under a directory (local repositories, single servers).
#[derive(Debug)]
pub struct FsBackend {
    dir: PathBuf,
}

impl FsBackend {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl Backend for FsBackend {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        match fs::read(self.dir.join(key)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        let path = self.dir.join(key);
        if path.exists() {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(&self.dir);
        fs::create_dir_all(parent)?;
        let written = write_atomic(&path, |file| file.write_all(bytes));
        // Another process may have stored the same object concurrently.
        match written {
            Ok(()) => Ok(()),
            Err(_) if path.exists() => Ok(()),
            Err(Error::Io { source, .. }) => Err(source),
            Err(e) => Err(std::io::Error::other(e.to_string())),
        }
    }

    fn exists(&self, key: &str) -> std::io::Result<bool> {
        Ok(self.dir.join(key).is_file())
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    backend: Arc<dyn Backend>,
    keys: Arc<Vec<Key>>,
}

/// The backend key of an object: `ab/cdef…`.
#[must_use]
pub fn key_of(id: &ObjectId) -> String {
    let hex = id.hex();
    format!("{}/{}", &hex[..2], &hex[2..])
}

fn io_error(id: &ObjectId, source: std::io::Error) -> Error {
    Error::Io {
        path: PathBuf::from(key_of(id)),
        source,
    }
}

impl Store {
    /// A plain store of files under `dir`.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self::with_backend(Arc::new(FsBackend::new(dir)), Vec::new())
    }

    /// A store that encrypts new objects with `keys[0]` and can read objects
    /// written with any of `keys` (or unencrypted).
    #[must_use]
    pub fn with_keys(dir: PathBuf, keys: Vec<Key>) -> Self {
        Self::with_backend(Arc::new(FsBackend::new(dir)), keys)
    }

    #[must_use]
    pub fn with_backend(backend: Arc<dyn Backend>, keys: Vec<Key>) -> Self {
        Self {
            backend,
            keys: Arc::new(keys),
        }
    }

    fn seal(&self, id: &ObjectId, raw: Vec<u8>) -> Result<Vec<u8>> {
        let Some(key) = self.keys.first() else {
            return Ok(raw);
        };
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce);
        let sealed = key
            .cipher
            .encrypt(
                &chacha20poly1305::XNonce::from(nonce),
                Payload {
                    msg: &raw,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| Error::InvalidArgument("encryption failed".to_owned()))?;
        let mut out = Vec::with_capacity(MAGIC.len() + KEY_ID_LEN + NONCE_LEN + sealed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&key.id);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    fn open(&self, id: &ObjectId, file: Vec<u8>) -> Result<Vec<u8>> {
        let Some(rest) = file.strip_prefix(MAGIC) else {
            return Ok(file);
        };
        if rest.len() < KEY_ID_LEN + NONCE_LEN {
            return Err(Error::CorruptObject(id.hex()));
        }
        let (key_id, rest) = rest.split_at(KEY_ID_LEN);
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        let key = self.keys.iter().find(|k| k.id == key_id).ok_or_else(|| {
            Error::InvalidArgument(format!(
                "object {} needs encryption key {}",
                id.short(),
                hex::encode(key_id)
            ))
        })?;
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| Error::CorruptObject(id.hex()))?;
        key.cipher
            .decrypt(
                &chacha20poly1305::XNonce::from(nonce),
                Payload {
                    msg: sealed,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| Error::CorruptObject(id.hex()))
    }

    pub fn write(&self, kind: &str, data: &[u8]) -> Result<ObjectId> {
        let id = ObjectId::hash(kind, data);
        let key = key_of(&id);
        if self.backend.exists(&key).map_err(|e| io_error(&id, e))? {
            return Ok(id);
        }
        let mut raw = Vec::with_capacity(kind.len() + 1 + data.len());
        raw.extend_from_slice(kind.as_bytes());
        raw.push(0);
        raw.extend_from_slice(data);
        let bytes = self.seal(&id, raw)?;
        self.backend
            .put(&key, &bytes)
            .map_err(|e| io_error(&id, e))?;
        Ok(id)
    }

    pub fn read(&self, kind: &str, id: &ObjectId) -> Result<Vec<u8>> {
        let raw = self.read_raw(id)?;
        let prefix_len = kind.len() + 1;
        if raw.len() < prefix_len || &raw[..kind.len()] != kind.as_bytes() || raw[kind.len()] != 0 {
            return Err(Error::CorruptObject(id.hex()));
        }
        Ok(raw[prefix_len..].to_vec())
    }

    /// Whether the object is stored. Backend errors count as absent.
    #[must_use]
    pub fn has(&self, id: &ObjectId) -> bool {
        self.backend.exists(&key_of(id)).unwrap_or(false)
    }

    /// The plaintext object bytes (kind, NUL, data), verified against the id.
    pub fn read_raw(&self, id: &ObjectId) -> Result<Vec<u8>> {
        let file = self
            .backend
            .get(&key_of(id))
            .map_err(|e| io_error(id, e))?
            .ok_or_else(|| {
                io_error(
                    id,
                    std::io::Error::new(std::io::ErrorKind::NotFound, "object not found"),
                )
            })?;
        let raw = self.open(id, file)?;
        if blake3::hash(&raw).as_bytes() != id.as_bytes() {
            return Err(Error::CorruptObject(id.hex()));
        }
        Ok(raw)
    }

    /// Stores raw object bytes received from another repository.
    pub fn write_raw(&self, raw: &[u8]) -> Result<ObjectId> {
        let nul = raw
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::InvalidArgument("object without kind".to_owned()))?;
        let kind = std::str::from_utf8(&raw[..nul])
            .map_err(|_| Error::InvalidArgument("object kind".to_owned()))?;
        if ![BLOB, TREE, COMMIT].contains(&kind) {
            return Err(Error::InvalidArgument(format!(
                "unknown object kind `{kind}`"
            )));
        }
        self.write(kind, &raw[nul + 1..])
    }

    pub fn write_json<T: Serialize>(&self, kind: &str, value: &T) -> Result<ObjectId> {
        self.write(kind, &serde_json::to_vec(value)?)
    }

    pub fn read_json<T: DeserializeOwned>(&self, kind: &str, id: &ObjectId) -> Result<T> {
        Ok(serde_json::from_slice(&self.read(kind, id)?)?)
    }

    /// Stores a tree as one object per directory, so directories that did
    /// not change are shared between versions and never sent twice. The
    /// empty tree keeps its original encoding (the root commit points at it).
    pub fn write_tree(&self, tree: &Tree) -> Result<ObjectId> {
        if tree.files.is_empty() {
            return self.write_json(TREE, &Tree::default());
        }
        let files = tree.files.iter().map(|(p, e)| (p.as_str(), e)).collect();
        self.write_dir(files)
    }

    fn write_dir(&self, files: Vec<(&str, &FileEntry)>) -> Result<ObjectId> {
        let mut dir = StoredTree {
            v: TREE_VERSION,
            ..StoredTree::default()
        };
        let mut subdirs: BTreeMap<&str, Vec<(&str, &FileEntry)>> = BTreeMap::new();
        for (path, entry) in files {
            match path.split_once('/') {
                Some((name, rest)) => subdirs.entry(name).or_default().push((rest, entry)),
                None => {
                    dir.files.insert(path.to_owned(), *entry);
                }
            }
        }
        for (name, files) in subdirs {
            let id = self.write_dir(files)?;
            dir.dirs.insert(name.to_owned(), id);
        }
        self.write_json(TREE, &dir)
    }

    /// Reads a tree in either format (flat or one object per directory).
    pub fn read_tree(&self, id: &ObjectId) -> Result<Tree> {
        let mut tree = Tree::default();
        self.read_dir(id, "", 0, &mut tree)?;
        Ok(tree)
    }

    fn read_dir(&self, id: &ObjectId, prefix: &str, depth: usize, out: &mut Tree) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            return Err(Error::InvalidArgument("tree is too deep".to_owned()));
        }
        let dir: StoredTree = self.read_json(TREE, id)?;
        if dir.v == 0 && depth == 0 {
            out.files = dir.files;
            return Ok(());
        }
        for (name, entry) in dir.files {
            check_name(&name)?;
            out.files.insert(format!("{prefix}{name}"), entry);
        }
        for (name, sub) in dir.dirs {
            check_name(&name)?;
            self.read_dir(&sub, &format!("{prefix}{name}/"), depth + 1, out)?;
        }
        Ok(())
    }

    /// Every object a tree needs (its directory objects and blobs), skipping
    /// those in `seen` and adding the rest to it.
    pub fn tree_objects(
        &self,
        id: &ObjectId,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<ObjectId>,
    ) -> Result<()> {
        self.walk_dir(id, 0, seen, out)
    }

    fn walk_dir(
        &self,
        id: &ObjectId,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<ObjectId>,
    ) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            return Err(Error::InvalidArgument("tree is too deep".to_owned()));
        }
        if !seen.insert(*id) {
            return Ok(());
        }
        out.push(*id);
        let dir: StoredTree = self.read_json(TREE, id)?;
        for entry in dir.files.values() {
            if seen.insert(entry.blob) {
                out.push(entry.blob);
            }
        }
        if dir.v == 0 && depth == 0 {
            return Ok(());
        }
        for sub in dir.dirs.values() {
            self.walk_dir(sub, depth + 1, seen, out)?;
        }
        Ok(())
    }
}

/// A file or directory name inside a stored directory.
fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(Error::InvalidPath(name.to_owned()));
    }
    Ok(())
}

/// Writes a file atomically: temporary file in the same directory, `fsync`,
/// then rename over the destination.
pub(crate) fn write_atomic(
    path: &Path,
    fill: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::InvalidPath(path.display().to_string()))?;
    let mut suffix = [0u8; 8];
    rand::fill(&mut suffix);
    let tmp = dir.join(format!(".tmp-{}", hex::encode(suffix)));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        fill(&mut file)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.at(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_objects_round_trip_and_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let old = Key::from_hex(&Key::generate_hex()).unwrap();
        let new = Key::from_hex(&Key::generate_hex()).unwrap();
        let store = Store::with_keys(dir.path().to_path_buf(), vec![old.clone()]);
        let id = store.write(BLOB, b"secret source").unwrap();

        // On disk it is not readable.
        let path = dir.path().join(key_of(&id));
        let file = fs::read(&path).unwrap();
        assert!(file.starts_with(MAGIC));
        assert!(!file.windows(13).any(|w| w == b"secret source"));

        // After rotation (new key first), old objects still read.
        let rotated = Store::with_keys(dir.path().to_path_buf(), vec![new, old]);
        assert_eq!(rotated.read(BLOB, &id).unwrap(), b"secret source");

        // Without the key, reading fails instead of returning garbage.
        let plain = Store::new(dir.path().to_path_buf());
        assert!(plain.read(BLOB, &id).is_err());
    }

    #[test]
    fn trees_are_stored_per_directory_and_share_unchanged_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let blob = store.write(BLOB, b"x").unwrap();
        let mut tree = Tree::default();
        for path in ["README", "src/lib.rs", "src/a/b.rs", "docs/guide.md"] {
            tree.files
                .insert(path.to_owned(), FileEntry::file(blob, false));
        }
        let id = store.write_tree(&tree).unwrap();
        assert_eq!(store.read_tree(&id).unwrap(), tree);

        let mut seen = HashSet::new();
        let mut first = Vec::new();
        store.tree_objects(&id, &mut seen, &mut first).unwrap();
        // root, src, src/a, docs and the one blob.
        assert_eq!(first.len(), 5);

        // Changing one file only adds the directories on its path.
        let edited = store.write(BLOB, b"y").unwrap();
        tree.files
            .insert("docs/guide.md".to_owned(), FileEntry::file(edited, false));
        let id2 = store.write_tree(&tree).unwrap();
        let mut second = Vec::new();
        store.tree_objects(&id2, &mut seen, &mut second).unwrap();
        assert_eq!(second.len(), 3, "new root, new docs and its new blob");
    }

    #[test]
    fn flat_trees_from_older_versions_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let blob = store.write(BLOB, b"x").unwrap();
        let mut tree = Tree::default();
        tree.files
            .insert("a/b/c.txt".to_owned(), FileEntry::file(blob, true));
        let flat = store.write_json(TREE, &tree).unwrap();
        assert_eq!(store.read_tree(&flat).unwrap(), tree);
        let empty = store.write_tree(&Tree::default()).unwrap();
        assert_eq!(empty, store.write_json(TREE, &Tree::default()).unwrap());
    }

    #[test]
    fn tampering_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let key = Key::from_hex(&Key::generate_hex()).unwrap();
        let store = Store::with_keys(dir.path().to_path_buf(), vec![key]);
        let id = store.write(BLOB, b"data").unwrap();
        let path = dir.path().join(key_of(&id));
        let mut file = fs::read(&path).unwrap();
        let last = file.len() - 1;
        file[last] ^= 1;
        fs::write(&path, file).unwrap();
        assert!(store.read(BLOB, &id).is_err());
    }
}
