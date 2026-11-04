//! Content-addressed object store.
//!
//! Each object lives in `objects/<2 hex>/<62 hex>`. Its id is the BLAKE3 hash
//! of `<kind>` + NUL + `<data>`, so every read can be verified.
//!
//! With encryption keys (servers), files hold `HBE1 | key id (8) | nonce (24)
//! | XChaCha20-Poly1305 ciphertext` and the object id is bound to the
//! ciphertext as associated data, so encrypted files cannot be swapped. The
//! first key encrypts; every key can decrypt, which allows key rotation.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, IoContext, Result};
use crate::ids::ObjectId;
use crate::model::{FileEntry, StoredTree, TREE_VERSION, Tree};
use crate::pack::{self, PackBuilder, PackSet};

pub const BLOB: &str = "blob";
pub const TREE: &str = "tree";
pub const COMMIT: &str = "commit";

const MAGIC: &[u8; 4] = b"HBE1";
/// Objects smaller than this are not worth compressing one by one.
const MIN_COMPRESS: usize = 96;
/// Fast, for objects written while working.
const LOOSE_LEVEL: u8 = 1;
/// Slower but smaller, for `hb gc`.
const PACK_LEVEL: u8 = 6;
/// Earlier versions each object is compared against (Git uses 10, across all
/// objects of a kind; ours are already grouped by path).
const DELTA_WINDOW: usize = 4;
/// Very large files are stored whole: comparing them costs more than it saves.
const MAX_DELTA_INPUT: usize = 64 * 1024 * 1024;
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
    /// Starts collecting new objects to write them together (nestable).
    fn begin_batch(&self) {}
    /// Ends a batch: writes it durably if `keep`, or drops it.
    fn end_batch(&self, _keep: bool) -> std::io::Result<()> {
        Ok(())
    }
}

/// Batches smaller than this are written as loose files: a pack per
/// command would leave thousands of tiny packs behind.
const MIN_PACK_OBJECTS: usize = 64;

#[derive(Debug, Default)]
struct Batch {
    depth: usize,
    builder: Option<PackBuilder>,
}

/// Objects as files under a directory (local repositories, single servers):
/// loose files `ab/cdef…`, plus pack files under `pack/`.
#[derive(Debug)]
pub struct FsBackend {
    dir: PathBuf,
    packs: PackSet,
    batch: Mutex<Batch>,
}

/// What [`FsBackend::repack`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RepackStats {
    /// Objects in the new pack.
    pub packed: usize,
    /// Loose files removed because the new pack has them.
    pub loose_removed: usize,
    /// Unreachable objects dropped.
    pub pruned: usize,
    /// Older packs replaced by the new one.
    pub packs_removed: usize,
}

impl FsBackend {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            packs: PackSet::default(),
            batch: Mutex::new(Batch::default()),
        }
    }

    fn batch(&self) -> std::io::Result<std::sync::MutexGuard<'_, Batch>> {
        self.batch
            .lock()
            .map_err(|_| std::io::Error::other("object batch lock poisoned"))
    }

    fn put_loose(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
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

    /// Adds `id` to the current batch as a delta (already compressed)
    /// against `base`. Only when `base` is in the same batch: a pack never
    /// points outside itself. Returns whether it was added.
    fn put_delta(
        &self,
        id: &ObjectId,
        base: &ObjectId,
        delta: &[u8],
        pending: bool,
    ) -> std::io::Result<bool> {
        let mut batch = self.batch()?;
        if batch.depth == 0 {
            return Ok(false);
        }
        if batch.builder.is_none() && pending {
            batch.builder = Some(PackBuilder::create(&self.dir)?);
        }
        let Some(builder) = batch.builder.as_mut() else {
            return Ok(false);
        };
        if !pending && !builder.contains(base.as_bytes()) {
            return Ok(false);
        }
        builder.put_delta(*id.as_bytes(), base.as_bytes(), delta)?;
        Ok(true)
    }

    fn stats(&self) -> StoreStats {
        let mut stats = StoreStats::default();
        for (key, _) in self.loose() {
            stats.loose_objects += 1;
            stats.loose_bytes += fs::metadata(self.dir.join(key)).map_or(0, |m| m.len());
        }
        self.packs.refresh(&self.dir, true);
        for pack in self.packs.all() {
            stats.packs += 1;
            stats.packed_objects += pack.ids().count();
            stats.pack_bytes += fs::metadata(&pack.pack_path).map_or(0, |m| m.len());
        }
        stats
    }

    /// Keys of every loose object file, with its modification time.
    fn loose(&self) -> Vec<(String, SystemTime)> {
        let mut out = Vec::new();
        let Ok(dirs) = fs::read_dir(&self.dir) else {
            return out;
        };
        for dir in dirs.flatten() {
            let name = dir.file_name().to_string_lossy().into_owned();
            if name.len() != 2 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(files) = fs::read_dir(dir.path()) else {
                continue;
            };
            for file in files.flatten() {
                let rest = file.file_name().to_string_lossy().into_owned();
                if rest.len() != 62 || !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                let mtime = file
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or_else(|_| SystemTime::now());
                out.push((format!("{name}/{rest}"), mtime));
            }
        }
        out
    }

    /// Rewrites every object in `keep` (that is stored here) into one new
    /// pack, then removes what the pack replaces: loose copies, older packs,
    /// and unreachable objects last written before `prune_before`. Newer
    /// unreachable objects stay, since a command may be about to use them.
    ///
    /// `families` lists objects likely to resemble each other (the versions
    /// of one path, newest first): each is stored as a delta against an
    /// earlier one when that saves space, like Git's pack heuristic.
    pub fn repack(
        &self,
        keep: &HashSet<ObjectId>,
        families: &[Vec<ObjectId>],
        prune_before: SystemTime,
    ) -> std::io::Result<RepackStats> {
        let mut stats = RepackStats::default();
        self.packs.refresh(&self.dir, true);
        let old_packs = self.packs.all();
        let loose = self.loose();

        let builder = Mutex::new(PackBuilder::create(&self.dir)?);
        self.pack_families(&builder, keep, families)?;
        let mut builder = builder
            .into_inner()
            .map_err(|_| std::io::Error::other("pack lock poisoned"))?;
        let mut copy = |id: &pack::Id| -> std::io::Result<()> {
            if builder.contains(id) || !keep.contains(&ObjectId::from_bytes(*id)) {
                return Ok(());
            }
            if let Some(bytes) = self.get(&pack::id_key(id))? {
                builder.put(*id, &compressed(bytes))?;
            }
            Ok(())
        };
        for (key, _) in &loose {
            if let Some(id) = pack::key_id(key) {
                copy(&id)?;
            }
        }
        for p in &old_packs {
            for id in p.ids() {
                copy(&id)?;
            }
        }
        stats.packed = builder.len();
        let new_idx = builder.finish()?;
        self.packs.refresh(&self.dir, true);

        for (key, mtime) in &loose {
            let Some(id) = pack::key_id(key) else {
                continue;
            };
            let kept = keep.contains(&ObjectId::from_bytes(id));
            if (kept || *mtime < prune_before) && fs::remove_file(self.dir.join(key)).is_ok() {
                if kept {
                    stats.loose_removed += 1;
                } else {
                    stats.pruned += 1;
                }
            }
        }
        // An older pack goes once everything worth keeping is in the new
        // one: when it is old enough, or holds nothing unreachable.
        let mut gone = HashSet::new();
        for p in &old_packs {
            if Some(&p.idx_path) == new_idx.as_ref() {
                continue;
            }
            let old_enough = fs::metadata(&p.idx_path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < prune_before);
            let unreachable = p
                .ids()
                .filter(|id| !keep.contains(&ObjectId::from_bytes(*id)))
                .count();
            if old_enough || unreachable == 0 {
                gone.insert(p.idx_path.clone());
                stats.pruned += unreachable;
            }
        }
        drop(old_packs);
        self.packs.forget(&gone);
        for idx in &gone {
            // The index first, so no reader trusts a pack being removed.
            if fs::remove_file(idx).is_ok() {
                let _ = fs::remove_file(idx.with_extension("hbp"));
                stats.packs_removed += 1;
            }
        }
        // Leftovers of interrupted writes: temporaries and packs whose index
        // never appeared (or whose removal failed while in use).
        let pack_dir = self.dir.join(pack::PACK_DIR);
        for entry in fs::read_dir(&pack_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < prune_before);
            let orphan = path.extension().is_some_and(|x| x == "hbp")
                && !path.with_extension("idx").exists();
            if stale && (name.starts_with(".tmp-") || orphan) {
                let _ = fs::remove_file(path);
            }
        }
        for dir in fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if dir.file_name().len() == 2 {
                let _ = fs::remove_dir(dir.path());
            }
        }
        Ok(stats)
    }
    /// Packs each family on one of several threads: reading, comparing and
    /// compressing happen in parallel, and only the final write is shared.
    fn pack_families(
        &self,
        builder: &Mutex<PackBuilder>,
        keep: &HashSet<ObjectId>,
        families: &[Vec<ObjectId>],
    ) -> std::io::Result<()> {
        let next = AtomicUsize::new(0);
        let claimed: Mutex<HashSet<ObjectId>> = Mutex::new(HashSet::new());
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let work = || -> std::io::Result<()> {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(family) = families.get(i) else {
                    return Ok(());
                };
                self.pack_family(builder, keep, family, &claimed)?;
            }
        };
        std::thread::scope(|scope| {
            let handles: Vec<_> = (1..workers).map(|_| scope.spawn(work)).collect();
            let mut result = work();
            for handle in handles {
                let joined = handle
                    .join()
                    .unwrap_or_else(|_| Err(std::io::Error::other("a pack worker panicked")));
                if result.is_ok() {
                    result = joined;
                }
            }
            result
        })
    }

    fn pack_family(
        &self,
        builder: &Mutex<PackBuilder>,
        keep: &HashSet<ObjectId>,
        family: &[ObjectId],
        claimed: &Mutex<HashSet<ObjectId>>,
    ) -> std::io::Result<()> {
        let poisoned = || std::io::Error::other("pack lock poisoned");
        // The last few objects of this family, with their chain depth. Only
        // objects packed here serve as bases, which rules out cycles.
        let mut window: VecDeque<(ObjectId, Vec<u8>, usize)> = VecDeque::new();
        for id in family {
            if !keep.contains(id) || !claimed.lock().map_err(|_| poisoned())?.insert(*id) {
                continue;
            }
            let Some(stored) = self.get(&key_of(id))? else {
                continue;
            };
            if stored.starts_with(MAGIC) {
                builder
                    .lock()
                    .map_err(|_| poisoned())?
                    .put(*id.as_bytes(), &stored)?;
                continue;
            }
            let plain = if pack::is_deflated(&stored) {
                pack::inflate(&stored)?
            } else {
                stored
            };
            let whole = pack::deflate(&plain, PACK_LEVEL);
            let mut best: Option<(ObjectId, Vec<u8>, usize)> = None;
            if plain.len() <= MAX_DELTA_INPUT {
                for (base, base_plain, depth) in window.iter().rev() {
                    if *depth >= pack::MAX_DEPTH {
                        continue;
                    }
                    let Some(delta) = crate::delta::encode(base_plain, &plain) else {
                        continue;
                    };
                    let delta = pack::deflate(&delta, PACK_LEVEL);
                    let limit = best.as_ref().map_or(whole.len(), |(_, d, _)| d.len());
                    if delta.len() + 33 < limit {
                        best = Some((*base, delta, depth + 1));
                    }
                }
            }
            let depth = {
                let mut pack = builder.lock().map_err(|_| poisoned())?;
                match best {
                    Some((base, delta, depth)) => {
                        pack.put_delta(*id.as_bytes(), base.as_bytes(), &delta)?;
                        depth
                    }
                    None => {
                        pack.put(*id.as_bytes(), &whole)?;
                        0
                    }
                }
            };
            if plain.len() <= MAX_DELTA_INPUT {
                window.push_back((*id, plain, depth));
                if window.len() > DELTA_WINDOW {
                    window.pop_front();
                }
            }
        }
        Ok(())
    }
}

impl Backend for FsBackend {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        match fs::read(self.dir.join(key)) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let Some(id) = pack::key_id(key) else {
            return Ok(None);
        };
        if let Some(builder) = self.batch()?.builder.as_mut()
            && let Some(bytes) = builder.get(&id)?
        {
            return Ok(Some(bytes));
        }
        self.packs.get(&self.dir, &id)
    }

    fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut batch = self.batch()?;
        if batch.depth > 0 {
            let id = pack::key_id(key)
                .ok_or_else(|| std::io::Error::other(format!("invalid object key {key}")))?;
            if batch.builder.is_none() {
                batch.builder = Some(PackBuilder::create(&self.dir)?);
            }
            if let Some(builder) = batch.builder.as_mut() {
                builder.put(id, bytes)?;
            }
            return Ok(());
        }
        drop(batch);
        self.put_loose(key, bytes)
    }

    fn exists(&self, key: &str) -> std::io::Result<bool> {
        // What is in memory first: a file check costs a system call, and
        // imports ask for every directory of every commit.
        if let Some(id) = pack::key_id(key)
            && (self
                .batch()?
                .builder
                .as_ref()
                .is_some_and(|b| b.contains(&id))
                || self.packs.contains(&self.dir, &id))
        {
            return Ok(true);
        }
        Ok(self.dir.join(key).is_file())
    }

    fn begin_batch(&self) {
        if let Ok(mut batch) = self.batch() {
            batch.depth += 1;
        }
    }

    fn end_batch(&self, keep: bool) -> std::io::Result<()> {
        let builder = {
            let mut batch = self.batch()?;
            batch.depth = batch.depth.saturating_sub(1);
            if batch.depth > 0 {
                return Ok(());
            }
            batch.builder.take()
        };
        let Some(builder) = builder else {
            return Ok(());
        };
        if !keep {
            builder.abort();
        } else if builder.len() < MIN_PACK_OBJECTS {
            for (id, bytes) in builder.drain()? {
                self.put_loose(&pack::id_key(&id), &bytes)?;
            }
        } else {
            builder.finish()?;
            self.packs.refresh(&self.dir, true);
        }
        Ok(())
    }
}

/// What an object directory holds (`hb stats`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StoreStats {
    pub loose_objects: usize,
    pub loose_bytes: u64,
    pub packs: usize,
    pub packed_objects: usize,
    pub pack_bytes: u64,
}

/// One path that differs between two trees: its entry on each side.
pub type TreeChange = (String, Option<FileEntry>, Option<FileEntry>);

/// Gets objects that are not here from somewhere else, for partial clones.
/// Returns them as raw bytes (kind, NUL, data); every one is verified.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, ids: &[ObjectId]) -> std::io::Result<Vec<Vec<u8>>>;
}

#[derive(Clone)]
pub struct Store {
    backend: Arc<dyn Backend>,
    /// The same backend when it is a local directory (for `repack`).
    local: Option<Arc<FsBackend>>,
    keys: Arc<Vec<Key>>,
    fetcher: Arc<OnceLock<Box<dyn Fetcher>>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
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
        Self::with_keys(dir, Vec::new())
    }

    /// A store that encrypts new objects with `keys[0]` and can read objects
    /// written with any of `keys` (or unencrypted).
    #[must_use]
    pub fn with_keys(dir: PathBuf, keys: Vec<Key>) -> Self {
        let local = Arc::new(FsBackend::new(dir));
        Self {
            backend: Arc::clone(&local) as Arc<dyn Backend>,
            local: Some(local),
            keys: Arc::new(keys),
            fetcher: Arc::new(OnceLock::new()),
        }
    }

    #[must_use]
    pub fn with_backend(backend: Arc<dyn Backend>, keys: Vec<Key>) -> Self {
        Self {
            backend,
            local: None,
            keys: Arc::new(keys),
            fetcher: Arc::new(OnceLock::new()),
        }
    }

    /// Objects missing here will be asked of `fetcher` when read.
    pub fn set_fetcher(&self, fetcher: Box<dyn Fetcher>) {
        let _ = self.fetcher.set(fetcher);
    }

    /// Whether objects may legitimately be missing (a partial clone).
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.fetcher.get().is_some()
    }

    /// Asks for many missing objects at once: reading them one by one would
    /// be one request each.
    pub fn prefetch(&self, ids: &[ObjectId]) -> Result<()> {
        let Some(fetcher) = self.fetcher.get() else {
            return Ok(());
        };
        let missing: Vec<ObjectId> = ids.iter().filter(|id| !self.has(id)).copied().collect();
        if missing.is_empty() {
            return Ok(());
        }
        let found = fetcher.fetch(&missing).map_err(|source| Error::Io {
            path: PathBuf::from("remote objects"),
            source,
        })?;
        for raw in found {
            self.write_raw(&raw)?;
        }
        Ok(())
    }

    /// How much is stored, for local stores.
    pub fn stats(&self) -> Result<StoreStats> {
        let local = self.local.as_ref().ok_or_else(|| {
            Error::InvalidArgument("only local object stores report their size".to_owned())
        })?;
        Ok(local.stats())
    }

    /// Whether the store has grown enough to compact (`hb gc --auto`). Like
    /// Git, it counts one of the 256 directories instead of all of them, so
    /// asking costs nothing.
    #[must_use]
    pub fn needs_gc(&self) -> bool {
        /// Git's `gc.auto` default, for one directory.
        const LOOSE_IN_ONE_DIR: usize = 6700 / 256;
        self.local.as_ref().is_some_and(|l| {
            fs::read_dir(l.dir.join("17")).is_ok_and(|d| d.count() > LOOSE_IN_ONE_DIR)
        })
    }

    /// Compacts a local store into one pack; see [`FsBackend::repack`].
    pub fn repack(
        &self,
        keep: &HashSet<ObjectId>,
        families: &[Vec<ObjectId>],
        prune_before: SystemTime,
    ) -> Result<RepackStats> {
        let local = self.local.as_ref().ok_or_else(|| {
            Error::InvalidArgument("only local object stores can be repacked".to_owned())
        })?;
        local
            .repack(keep, families, prune_before)
            .map_err(|source| Error::Io {
                path: PathBuf::from("objects"),
                source,
            })
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

    /// The plain object from the bytes a backend returned.
    fn open(&self, id: &ObjectId, file: Vec<u8>) -> Result<Vec<u8>> {
        let opened = self.decrypt(id, file)?;
        if pack::is_deflated(&opened) {
            return pack::inflate(&opened).map_err(|_| Error::CorruptObject(id.hex()));
        }
        Ok(opened)
    }

    fn decrypt(&self, id: &ObjectId, file: Vec<u8>) -> Result<Vec<u8>> {
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

    /// Runs `f` with new objects collected into one pack file (or loose
    /// files, for a few), made durable before this returns. Use it around
    /// anything that writes many objects, before recording the operation.
    pub fn batch<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.backend.begin_batch();
        let out = f();
        let ended = self.backend.end_batch(out.is_ok());
        let out = out?;
        ended.map_err(|source| Error::Io {
            path: PathBuf::from("objects/pack"),
            source,
        })?;
        Ok(out)
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
        let bytes = self.seal(&id, shrink(raw))?;
        self.backend
            .put(&key, &bytes)
            .map_err(|e| io_error(&id, e))?;
        Ok(id)
    }

    /// Stores an object whose id the caller already computed
    /// (`ObjectId::hash(kind, data)`), compressed as for a pack. For bulk
    /// imports, where every object is new.
    pub fn put_object(&self, id: &ObjectId, kind: &str, data: &[u8]) -> Result<()> {
        let mut raw = Vec::with_capacity(kind.len() + 1 + data.len());
        raw.extend_from_slice(kind.as_bytes());
        raw.push(0);
        raw.extend_from_slice(data);
        let bytes = self.seal(id, compressed(raw))?;
        self.backend
            .put(&key_of(id), &bytes)
            .map_err(|e| io_error(id, e))
    }

    /// Stores `id` as a delta against `base` (see [`crate::delta`]; both
    /// are whole objects, kind and NUL included), when that is possible: a
    /// local, unencrypted store inside [`Self::batch`] that already wrote
    /// `base` in the same batch. Returns whether it did; if not, the caller
    /// stores the object whole.
    pub fn put_delta(&self, id: &ObjectId, base: &ObjectId, delta: &[u8]) -> Result<bool> {
        self.put_delta_inner(id, base, delta, false)
    }

    /// Like [`Self::put_delta`] when `base` is not written yet but will be,
    /// in this same batch, before it ends (writers working in parallel).
    pub fn put_delta_pending(&self, id: &ObjectId, base: &ObjectId, delta: &[u8]) -> Result<bool> {
        self.put_delta_inner(id, base, delta, true)
    }

    fn put_delta_inner(
        &self,
        id: &ObjectId,
        base: &ObjectId,
        delta: &[u8],
        pending: bool,
    ) -> Result<bool> {
        let Some(local) = self.local.as_ref() else {
            return Ok(false);
        };
        if !self.keys.is_empty() {
            return Ok(false);
        }
        let packed = pack::deflate(delta, PACK_LEVEL);
        local
            .put_delta(id, base, &packed, pending)
            .map_err(|e| io_error(id, e))
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
        let mut file = self.backend.get(&key_of(id)).map_err(|e| io_error(id, e))?;
        if file.is_none() && self.is_partial() {
            self.prefetch(std::slice::from_ref(id))?;
            file = self.backend.get(&key_of(id)).map_err(|e| io_error(id, e))?;
        }
        let file = file.ok_or_else(|| {
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

    /// Whether two trees hold the same files. Different ids can only mean
    /// the same files when one of them is in the old flat format.
    pub fn same_tree(&self, a: &ObjectId, b: &ObjectId) -> Result<bool> {
        if a == b {
            return Ok(true);
        }
        let flat =
            |id: &ObjectId| -> Result<bool> { Ok(self.read_json::<StoredTree>(TREE, id)?.v == 0) };
        if flat(a)? || flat(b)? {
            return Ok(self.read_tree(a)? == self.read_tree(b)?);
        }
        Ok(false)
    }

    /// The paths that differ between two trees, each with its entry in `to`
    /// (`None` when deleted), sorted by path. Directories with the same id
    /// on both sides are skipped without being read.
    pub fn tree_changes(
        &self,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Result<Vec<(String, Option<FileEntry>)>> {
        Ok(self
            .tree_delta(from, to)?
            .into_iter()
            .map(|(path, _, new)| (path, new))
            .collect())
    }

    /// Like [`Self::tree_changes`], with the entry on each side.
    pub fn tree_delta(&self, from: &ObjectId, to: &ObjectId) -> Result<Vec<TreeChange>> {
        let mut out = Vec::new();
        if from != to {
            self.diff_dirs(Some(from), Some(to), "", 0, &mut out, &mut |_, _| {})?;
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Calls `visit` with the path and object of everything `to` has that
    /// `from` does not: new blobs, and directories (their paths ending in
    /// `/`). What `hb gc` uses to line up the versions of each path.
    pub fn walk_changes(
        &self,
        from: Option<&ObjectId>,
        to: &ObjectId,
        visit: &mut dyn FnMut(String, ObjectId),
    ) -> Result<()> {
        if from == Some(to) {
            return Ok(());
        }
        let mut out = Vec::new();
        visit("/".to_owned(), *to);
        self.diff_dirs(from, Some(to), "", 0, &mut out, visit)?;
        for (path, _, new) in out {
            if let Some(entry) = new {
                visit(path, entry.blob);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn diff_dirs(
        &self,
        from: Option<&ObjectId>,
        to: Option<&ObjectId>,
        prefix: &str,
        depth: usize,
        out: &mut Vec<TreeChange>,
        visit_dir: &mut dyn FnMut(String, ObjectId),
    ) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            return Err(Error::InvalidArgument("tree is too deep".to_owned()));
        }
        let read = |id: Option<&ObjectId>| -> Result<StoredTree> {
            id.map_or_else(|| Ok(StoredTree::default()), |id| self.read_json(TREE, id))
        };
        let (a, b) = (read(from)?, read(to)?);
        // Flat trees from older versions: compare them whole.
        if depth == 0 && (from.is_some() && a.v == 0 || to.is_some() && b.v == 0) {
            let a = from.map_or_else(|| Ok(Tree::default()), |id| self.read_tree(id))?;
            let b = to.map_or_else(|| Ok(Tree::default()), |id| self.read_tree(id))?;
            for (path, entry) in &a.files {
                if !b.files.contains_key(path) {
                    out.push((format!("{prefix}{path}"), Some(*entry), None));
                }
            }
            for (path, entry) in &b.files {
                let old = a.files.get(path);
                if old != Some(entry) {
                    out.push((format!("{prefix}{path}"), old.copied(), Some(*entry)));
                }
            }
            return Ok(());
        }
        for (name, entry) in &a.files {
            check_name(name)?;
            if !b.files.contains_key(name) {
                out.push((format!("{prefix}{name}"), Some(*entry), None));
            }
        }
        for (name, entry) in &b.files {
            check_name(name)?;
            let old = a.files.get(name);
            if old != Some(entry) {
                out.push((format!("{prefix}{name}"), old.copied(), Some(*entry)));
            }
        }
        let names: BTreeSet<&String> = a.dirs.keys().chain(b.dirs.keys()).collect();
        for name in names {
            check_name(name)?;
            let (x, y) = (a.dirs.get(name), b.dirs.get(name));
            if x != y {
                let path = format!("{prefix}{name}/");
                if let Some(id) = y {
                    visit_dir(path.clone(), *id);
                }
                self.diff_dirs(x, y, &path, depth + 1, out, visit_dir)?;
            }
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
        self.walk_dir(id, 0, true, seen, out)
    }

    /// Like [`Self::tree_objects`], only the directories.
    pub fn tree_dirs(
        &self,
        id: &ObjectId,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<ObjectId>,
    ) -> Result<()> {
        self.walk_dir(id, 0, false, seen, out)
    }

    fn walk_dir(
        &self,
        id: &ObjectId,
        depth: usize,
        blobs: bool,
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
        if blobs {
            for entry in dir.files.values() {
                if seen.insert(entry.blob) {
                    out.push(entry.blob);
                }
            }
        }
        if dir.v == 0 && depth == 0 {
            return Ok(());
        }
        for sub in dir.dirs.values() {
            self.walk_dir(sub, depth + 1, blobs, seen, out)?;
        }
        Ok(())
    }
}

/// Light compression for objects written while working.
fn shrink(raw: Vec<u8>) -> Vec<u8> {
    if raw.len() < MIN_COMPRESS {
        return raw;
    }
    let packed = pack::deflate(&raw, LOOSE_LEVEL);
    if packed.len() < raw.len() {
        packed
    } else {
        raw
    }
}

/// An object's stored form, compressed if it was not yet and that helps.
fn compressed(bytes: Vec<u8>) -> Vec<u8> {
    if pack::is_deflated(&bytes) || bytes.starts_with(MAGIC) || bytes.len() < MIN_COMPRESS {
        return bytes;
    }
    let packed = pack::deflate(&bytes, PACK_LEVEL);
    if packed.len() < bytes.len() {
        packed
    } else {
        bytes
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
    write_replacing(path, true, fill)
}

/// Like [`write_atomic`], for files that can always be rebuilt: a crash may
/// lose the latest version, which is fine, and skipping the sync keeps every
/// command from waiting for the disk.
pub(crate) fn write_cache(
    path: &Path,
    fill: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<()> {
    write_replacing(path, false, fill)
}

fn write_replacing(
    path: &Path,
    durable: bool,
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
        if durable {
            file.sync_all()?;
        }
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

    fn loose_files(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|d| d.file_name().len() == 2)
            .map(|d| fs::read_dir(d.path()).unwrap().count())
            .sum()
    }

    #[test]
    fn batches_become_packs_readable_by_other_stores() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let ids = store
            .batch(|| {
                (0..200)
                    .map(|i| store.write(BLOB, format!("object {i}").as_bytes()))
                    .collect::<Result<Vec<_>>>()
            })
            .unwrap();
        assert_eq!(loose_files(dir.path()), 0, "one pack, no loose files");
        // Another process (a new store) finds them in the pack.
        let other = Store::new(dir.path().to_path_buf());
        assert_eq!(other.read(BLOB, &ids[123]).unwrap(), b"object 123");
        assert!(other.has(&ids[0]));

        // Small batches stay loose; failed batches leave nothing behind.
        store.batch(|| store.write(BLOB, b"small")).unwrap();
        assert_eq!(loose_files(dir.path()), 1);
        let failed: Result<()> = store.batch(|| {
            store.write(BLOB, b"never kept")?;
            Err(Error::InvalidArgument("boom".to_owned()))
        });
        assert!(failed.is_err());
        assert!(!store.has(&ObjectId::hash(BLOB, b"never kept")));
    }

    #[test]
    fn repack_keeps_reachable_objects_and_prunes_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let packed = store
            .batch(|| {
                (0..100)
                    .map(|i| store.write(BLOB, format!("p{i}").as_bytes()))
                    .collect::<Result<Vec<_>>>()
            })
            .unwrap();
        let loose_kept = store.write(BLOB, b"loose kept").unwrap();
        let loose_garbage = store.write(BLOB, b"loose garbage").unwrap();

        let mut keep: HashSet<ObjectId> = packed[..50].iter().copied().collect();
        keep.insert(loose_kept);
        // Nothing is old enough to prune yet: the garbage survives.
        let first = store.repack(&keep, &[], SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(first.packed, 51);
        assert!(store.has(&loose_garbage));
        assert!(store.has(&packed[99]), "the young pack with garbage stays");

        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        let second = store.repack(&keep, &[], later).unwrap();
        assert!(!store.has(&loose_garbage));
        assert!(!store.has(&packed[99]));
        assert_eq!(loose_files(dir.path()), 0);
        let fresh = Store::new(dir.path().to_path_buf());
        for id in &keep {
            assert!(fresh.has(id));
        }
        assert_eq!(fresh.read(BLOB, &packed[7]).unwrap(), b"p7");
        assert!(second.pruned >= 51);
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
