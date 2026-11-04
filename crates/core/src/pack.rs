//! Pack files: many objects in one file, so writing thousands of objects
//! (an import, a pull, a first snapshot) costs a few files instead of one
//! per object, and `hb gc` can compact a repository.
//!
//! `objects/pack/pack-<hash>.hbp` starts with `HBPACK2\n`, then for each
//! object its 32-byte id, the body length as a little-endian `u64` and the
//! body: a method byte, then either the object as the store keeps it
//! (`STORED`) or a base id and a compressed delta against it (`DELTA`; the
//! base is always in the same pack). `pack-<hash>.idx` lists the ids sorted,
//! each with offset and length. A pack only counts once its index exists,
//! and the index is written last, so readers never see half a pack.
//!
//! Version 1 packs (`HBPACK1\n`, bodies without a method byte) stay readable.
//!
//! Indexes are kept as raw bytes and searched in place, and nothing is read
//! until a lookup needs it: a command that touches three objects does not
//! pay for a million.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use crate::delta;

const PACK_MAGIC_V1: &[u8] = b"HBPACK1\n";
const PACK_MAGIC: &[u8] = b"HBPACK2\n";
const IDX_MAGIC: &[u8] = b"HBIDX1\n";
const IDX_ENTRY: usize = 32 + 8 + 8;
const IDX_HEADER: usize = 7 + 8;
pub(crate) const PACK_DIR: &str = "pack";

const STORED: u8 = 0;
const DELTA: u8 = 1;
/// Longer chains make reads slower for little gain (Git uses 50).
pub(crate) const MAX_DEPTH: usize = 32;
/// Decoded bases kept around while walking a chain.
const BASE_CACHE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) type Id = [u8; 32];

/// The binary id of a backend key (`ab/cdef…`).
pub(crate) fn key_id(key: &str) -> Option<Id> {
    let mut hex_id = String::with_capacity(64);
    for part in key.split('/') {
        hex_id.push_str(part);
    }
    hex::decode(hex_id).ok()?.try_into().ok()
}

pub(crate) fn id_key(id: &Id) -> String {
    let hex = hex::encode(id);
    format!("{}/{}", &hex[..2], &hex[2..])
}

/// Compresses in zlib format, recognizable by its first byte (`x`).
pub(crate) fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, level)
}

pub(crate) fn inflate(data: &[u8]) -> io::Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib(data)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("inflate: {e:?}")))
}

/// Whether bytes kept by the store are compressed. Objects start with their
/// kind (`blob`, `tree`, `commit`) and encrypted files with `HBE1`, so the
/// zlib header cannot be mistaken for either.
pub(crate) fn is_deflated(bytes: &[u8]) -> bool {
    bytes.first() == Some(&0x78)
}

/// One pack and its index.
#[derive(Debug)]
pub(crate) struct Pack {
    pub idx_path: PathBuf,
    pub pack_path: PathBuf,
    /// Sorted entries of the index, as on disk.
    index: Vec<u8>,
    /// Opened on first read; read at offsets, so threads never wait.
    file: std::sync::OnceLock<(File, u8)>,
    bases: Mutex<BaseCache>,
}

#[derive(Debug, Default)]
struct BaseCache {
    bytes: usize,
    entries: HashMap<Id, Arc<Vec<u8>>>,
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

impl Pack {
    fn load(idx_path: &Path) -> io::Result<Self> {
        let raw = fs::read(idx_path)?;
        let bad = || invalid("invalid pack index");
        let rest = raw.strip_prefix(IDX_MAGIC).ok_or_else(bad)?;
        let count = rest.get(..8).ok_or_else(bad)?;
        let count = u64::from_le_bytes(count.try_into().map_err(|_| bad())?);
        let count = usize::try_from(count).map_err(|_| bad())?;
        if rest.len() - 8 != count.checked_mul(IDX_ENTRY).ok_or_else(bad)? {
            return Err(bad());
        }
        let mut index = raw;
        index.drain(..IDX_HEADER);
        Ok(Self {
            idx_path: idx_path.to_path_buf(),
            pack_path: idx_path.with_extension("hbp"),
            index,
            file: std::sync::OnceLock::new(),
            bases: Mutex::new(BaseCache::default()),
        })
    }

    fn entry(&self, i: usize) -> &[u8] {
        &self.index[i * IDX_ENTRY..(i + 1) * IDX_ENTRY]
    }

    fn len(&self) -> usize {
        self.index.len() / IDX_ENTRY
    }

    fn find(&self, id: &Id) -> Option<(u64, u64)> {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let mid = low + (high - low) / 2;
            let entry = self.entry(mid);
            match entry[..32].cmp(id.as_slice()) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid,
                std::cmp::Ordering::Equal => {
                    let offset = entry[32..40].try_into().ok()?;
                    let len = entry[40..48].try_into().ok()?;
                    return Some((u64::from_le_bytes(offset), u64::from_le_bytes(len)));
                }
            }
        }
        None
    }

    pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.index.chunks_exact(IDX_ENTRY).filter_map(|e| {
            let id: Option<Id> = e[..32].try_into().ok();
            id
        })
    }

    /// The pack's format version and the body at this span.
    fn body(&self, offset: u64, len: u64) -> io::Result<(u8, Vec<u8>)> {
        if self.file.get().is_none() {
            let mut file = File::open(&self.pack_path)?;
            let mut magic = [0u8; 8];
            file.read_exact(&mut magic)?;
            let version = if magic == PACK_MAGIC {
                2
            } else if magic == PACK_MAGIC_V1 {
                1
            } else {
                return Err(invalid("invalid pack"));
            };
            let _ = self.file.set((file, version));
        }
        let (file, version) = self
            .file
            .get()
            .ok_or_else(|| io::Error::other("pack not open"))?;
        let len = usize::try_from(len).map_err(|_| io::Error::other("object too large"))?;
        let mut buf = vec![0; len];
        read_exact_at(file, &mut buf, offset)?;
        Ok((*version, buf))
    }

    /// The object at this span, as the store expects it: exactly as stored,
    /// or plain bytes when it was a delta.
    fn read(&self, offset: u64, len: u64, depth: usize) -> io::Result<Vec<u8>> {
        let (version, mut body) = self.body(offset, len)?;
        if version == 1 {
            return Ok(body);
        }
        match body.first().copied() {
            Some(STORED) => {
                body.remove(0);
                Ok(body)
            }
            Some(DELTA) if body.len() > 33 => {
                if depth > MAX_DEPTH + 8 {
                    return Err(invalid("delta chain too deep"));
                }
                let base: Id = body[1..33].try_into().map_err(|_| invalid("delta base"))?;
                let base = self.plain(&base, depth + 1)?;
                let instructions = inflate(&body[33..])?;
                delta::apply(&base, &instructions).ok_or_else(|| invalid("corrupt delta"))
            }
            _ => Err(invalid("invalid pack entry")),
        }
    }

    /// The uncompressed bytes of an object of this pack.
    fn plain(&self, id: &Id, depth: usize) -> io::Result<Arc<Vec<u8>>> {
        if let Ok(bases) = self.bases.lock()
            && let Some(hit) = bases.entries.get(id)
        {
            return Ok(Arc::clone(hit));
        }
        let (offset, len) = self
            .find(id)
            .ok_or_else(|| invalid("delta base missing from its pack"))?;
        let bytes = self.read(offset, len, depth)?;
        let plain = Arc::new(if is_deflated(&bytes) {
            inflate(&bytes)?
        } else {
            bytes
        });
        if let Ok(mut bases) = self.bases.lock() {
            if bases.bytes + plain.len() > BASE_CACHE_BYTES {
                bases.entries.clear();
                bases.bytes = 0;
            }
            bases.bytes += plain.len();
            bases.entries.insert(*id, Arc::clone(&plain));
        }
        Ok(plain)
    }
}

/// Reads exactly `buf.len()` bytes at `offset`, without moving a shared
/// cursor, so several threads can read one file at once.
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(file, buf, offset)?;
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(file, buf, offset)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "pack ends early",
            ));
        }
        buf = &mut buf[n..];
        offset += n as u64;
    }
    Ok(())
}

/// The packs of one object directory. Nothing is read until the first
/// lookup; new packs written by other processes are picked up when a lookup
/// misses and the directory has changed.
#[derive(Debug, Default)]
pub(crate) struct PackSet {
    packs: RwLock<Loaded>,
}

#[derive(Debug, Default)]
struct Loaded {
    ready: bool,
    list: Vec<Arc<Pack>>,
    stamp: Option<SystemTime>,
}

impl PackSet {
    fn ensure(&self, dir: &Path) {
        let ready = self.packs.read().is_ok_and(|l| l.ready);
        if !ready {
            self.refresh(dir, true);
        }
    }

    fn lookup(&self, id: &Id) -> Option<(Arc<Pack>, u64, u64)> {
        let loaded = self.packs.read().ok()?;
        loaded
            .list
            .iter()
            .find_map(|p| p.find(id).map(|(o, l)| (Arc::clone(p), o, l)))
    }

    /// Reloads the pack list if the pack directory changed since last time.
    pub fn refresh(&self, dir: &Path, force: bool) {
        let pack_dir = dir.join(PACK_DIR);
        let stamp = fs::metadata(&pack_dir).and_then(|m| m.modified()).ok();
        let Ok(mut loaded) = self.packs.write() else {
            return;
        };
        if !force && loaded.stamp == stamp {
            return;
        }
        loaded.ready = true;
        loaded.stamp = stamp;
        let mut current: Vec<PathBuf> = fs::read_dir(&pack_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "idx"))
                    .collect()
            })
            .unwrap_or_default();
        current.sort();
        loaded.list.retain(|p| current.contains(&p.idx_path));
        for idx in current {
            if loaded.list.iter().any(|p| p.idx_path == idx) {
                continue;
            }
            if let Ok(pack) = Pack::load(&idx) {
                loaded.list.push(Arc::new(pack));
            }
        }
    }

    pub fn get(&self, dir: &Path, id: &Id) -> io::Result<Option<Vec<u8>>> {
        self.ensure(dir);
        for attempt in 0..2 {
            if attempt == 1 {
                self.refresh(dir, false);
            }
            if let Some((pack, offset, len)) = self.lookup(id) {
                match pack.read(offset, len, 0) {
                    Ok(bytes) => return Ok(Some(bytes)),
                    // Removed by `gc` in another process: look again.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        self.refresh(dir, true);
                        if let Some((pack, offset, len)) = self.lookup(id) {
                            return pack.read(offset, len, 0).map(Some);
                        }
                        return Ok(None);
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(None)
    }

    pub fn contains(&self, dir: &Path, id: &Id) -> bool {
        self.ensure(dir);
        if self.lookup(id).is_some() {
            return true;
        }
        self.refresh(dir, false);
        self.lookup(id).is_some()
    }

    pub fn all(&self) -> Vec<Arc<Pack>> {
        self.packs
            .read()
            .map(|l| l.list.clone())
            .unwrap_or_default()
    }

    /// Forgets packs (closing their files) so they can be deleted.
    pub fn forget(&self, remove: &HashSet<PathBuf>) {
        if let Ok(mut loaded) = self.packs.write() {
            loaded.list.retain(|p| !remove.contains(&p.idx_path));
        }
    }
}

/// Objects being written into a new pack (a batch). Until it is finished
/// they are only visible to this process.
#[derive(Debug)]
pub(crate) struct PackBuilder {
    tmp: PathBuf,
    /// Buffered: a batch can hold a million small objects.
    file: io::BufWriter<File>,
    /// Whether a read moved the file away from its end.
    moved: bool,
    offset: u64,
    entries: BTreeMap<Id, (u64, u64)>,
}

impl PackBuilder {
    pub fn create(dir: &Path) -> io::Result<Self> {
        let pack_dir = dir.join(PACK_DIR);
        fs::create_dir_all(&pack_dir)?;
        let mut suffix = [0u8; 8];
        rand::fill(&mut suffix);
        let tmp = pack_dir.join(format!(".tmp-{}.hbp", hex::encode(suffix)));
        let mut file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(PACK_MAGIC)?;
        Ok(Self {
            tmp,
            file: io::BufWriter::with_capacity(1 << 20, file),
            moved: false,
            offset: PACK_MAGIC.len() as u64,
            entries: BTreeMap::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, id: &Id) -> bool {
        self.entries.contains_key(id)
    }

    fn entry(&mut self, id: Id, parts: &[&[u8]]) -> io::Result<()> {
        if self.entries.contains_key(&id) {
            return Ok(());
        }
        let len: usize = parts.iter().map(|p| p.len()).sum();
        let len = len as u64;
        if self.moved {
            self.file.get_mut().seek(SeekFrom::Start(self.offset))?;
            self.moved = false;
        }
        self.file.write_all(&id)?;
        self.file.write_all(&len.to_le_bytes())?;
        for part in parts {
            self.file.write_all(part)?;
        }
        let start = self.offset + 40;
        self.entries.insert(id, (start, len));
        self.offset = start + len;
        Ok(())
    }

    /// Adds an object exactly as the store keeps it.
    pub fn put(&mut self, id: Id, bytes: &[u8]) -> io::Result<()> {
        self.entry(id, &[&[STORED], bytes])
    }

    /// Adds an object as a delta (already compressed) against `base`, which
    /// must end up in this same pack.
    pub fn put_delta(&mut self, id: Id, base: &Id, delta: &[u8]) -> io::Result<()> {
        self.entry(id, &[&[DELTA], base, delta])
    }

    /// An object of this batch, as the store keeps it (plain bytes when it
    /// was written as a delta).
    pub fn get(&mut self, id: &Id) -> io::Result<Option<Vec<u8>>> {
        self.get_at(id, 0)
    }

    fn get_at(&mut self, id: &Id, depth: usize) -> io::Result<Option<Vec<u8>>> {
        let Some(&(offset, len)) = self.entries.get(id) else {
            return Ok(None);
        };
        self.file.flush()?;
        self.moved = true;
        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0; usize::try_from(len).map_err(|_| io::Error::other("too large"))?];
        file.read_exact(&mut buf)?;
        match buf.first().copied() {
            Some(STORED) => {
                buf.remove(0);
                Ok(Some(buf))
            }
            Some(DELTA) if buf.len() > 33 && depth <= MAX_DEPTH + 8 => {
                let base: Id = buf[1..33].try_into().map_err(|_| invalid("delta base"))?;
                let base = self
                    .get_at(&base, depth + 1)?
                    .ok_or_else(|| invalid("delta base missing from its pack"))?;
                let base = if is_deflated(&base) {
                    inflate(&base)?
                } else {
                    base
                };
                let instructions = inflate(&buf[33..])?;
                delta::apply(&base, &instructions)
                    .map(Some)
                    .ok_or_else(|| invalid("corrupt delta"))
            }
            _ => Err(invalid("invalid pack entry")),
        }
    }

    /// Every object, for writing a small batch as loose files instead.
    pub fn drain(mut self) -> io::Result<Vec<(Id, Vec<u8>)>> {
        let ids: Vec<Id> = self.entries.keys().copied().collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(bytes) = self.get(&id)? {
                out.push((id, bytes));
            }
        }
        self.abort();
        Ok(out)
    }

    pub fn abort(self) {
        let Self { tmp, file, .. } = self;
        drop(file);
        let _ = fs::remove_file(tmp);
    }

    /// Makes the pack durable and visible: data, then index. Returns the
    /// index path, or `None` if the batch was empty.
    pub fn finish(self) -> io::Result<Option<PathBuf>> {
        if self.entries.is_empty() {
            self.abort();
            return Ok(None);
        }
        let Self {
            tmp, file, entries, ..
        } = self;
        let file = file.into_inner().map_err(io::IntoInnerError::into_error)?;
        file.sync_all()?;
        drop(file);
        let mut idx = Vec::with_capacity(IDX_HEADER + entries.len() * IDX_ENTRY);
        idx.extend_from_slice(IDX_MAGIC);
        idx.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (id, (offset, len)) in &entries {
            idx.extend_from_slice(id);
            idx.extend_from_slice(&offset.to_le_bytes());
            idx.extend_from_slice(&len.to_le_bytes());
        }
        let name = &blake3::hash(&idx).to_hex()[..32];
        let dir = tmp.parent().unwrap_or(Path::new("."));
        let pack_path = dir.join(format!("pack-{name}.hbp"));
        let idx_path = dir.join(format!("pack-{name}.idx"));
        if idx_path.exists() {
            let _ = fs::remove_file(&tmp);
            return Ok(Some(idx_path));
        }
        fs::rename(&tmp, &pack_path)?;
        crate::store::write_atomic(&idx_path, |f| f.write_all(&idx))
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Some(idx_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_read_back_through_their_chain() {
        let dir = tempfile::tempdir().unwrap();
        let mut text = String::new();
        for i in 0..500 {
            text.push_str(&format!("line {i}\n"));
        }
        let first = text.clone().into_bytes();
        text.push_str("second version\n");
        let second = text.clone().into_bytes();
        text.push_str("third version\n");
        let third = text.clone().into_bytes();

        let ids: [Id; 3] = [[1; 32], [2; 32], [3; 32]];
        let mut builder = PackBuilder::create(dir.path()).unwrap();
        builder.put(ids[0], &deflate(&first, 6)).unwrap();
        let d = deflate(&delta::encode(&first, &second).unwrap(), 6);
        builder.put_delta(ids[1], &ids[0], &d).unwrap();
        let d = deflate(&delta::encode(&second, &third).unwrap(), 6);
        builder.put_delta(ids[2], &ids[1], &d).unwrap();
        builder.finish().unwrap();

        let set = PackSet::default();
        assert!(set.contains(dir.path(), &ids[2]));
        assert_eq!(set.get(dir.path(), &ids[2]).unwrap().unwrap(), third);
        assert_eq!(set.get(dir.path(), &ids[1]).unwrap().unwrap(), second);
        assert!(set.get(dir.path(), &[9; 32]).unwrap().is_none());
    }

    #[test]
    fn version_one_packs_stay_readable() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join(PACK_DIR);
        fs::create_dir_all(&pack_dir).unwrap();
        let id: Id = [7; 32];
        let body = b"blob\0old format";
        let mut pack = PACK_MAGIC_V1.to_vec();
        pack.extend_from_slice(&id);
        pack.extend_from_slice(&(body.len() as u64).to_le_bytes());
        pack.extend_from_slice(body);
        fs::write(pack_dir.join("pack-a.hbp"), pack).unwrap();
        let mut idx = IDX_MAGIC.to_vec();
        idx.extend_from_slice(&1u64.to_le_bytes());
        idx.extend_from_slice(&id);
        idx.extend_from_slice(&((PACK_MAGIC_V1.len() + 40) as u64).to_le_bytes());
        idx.extend_from_slice(&(body.len() as u64).to_le_bytes());
        fs::write(pack_dir.join("pack-a.idx"), idx).unwrap();

        let set = PackSet::default();
        assert_eq!(set.get(dir.path(), &id).unwrap().unwrap(), body);
    }
}
