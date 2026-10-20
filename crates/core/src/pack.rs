//! Pack files: many objects in one file, so writing thousands of objects
//! (an import, a pull, a first snapshot) costs a few files instead of one
//! per object, and `hb gc` can compact a repository.
//!
//! `objects/pack/pack-<hash>.hbp` holds `HBPACK1\n`, then for each object its
//! 32-byte id, its length as a little-endian `u64` and the bytes exactly as
//! the backend stores them (encrypted or not). `pack-<hash>.idx` lists the
//! ids sorted, each with offset and length. A pack only counts once its
//! index exists, and the index is written last, so readers never see half a
//! pack.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

const PACK_MAGIC: &[u8] = b"HBPACK1\n";
const IDX_MAGIC: &[u8] = b"HBIDX1\n";
const IDX_ENTRY: usize = 32 + 8 + 8;
pub(crate) const PACK_DIR: &str = "pack";

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

/// One pack and its index, loaded in memory (48 bytes per object).
#[derive(Debug)]
pub(crate) struct Pack {
    pub idx_path: PathBuf,
    pub pack_path: PathBuf,
    ids: Vec<Id>,
    spans: Vec<(u64, u64)>,
    file: Mutex<Option<File>>,
}

impl Pack {
    fn load(idx_path: &Path) -> io::Result<Self> {
        let raw = fs::read(idx_path)?;
        let bad = || io::Error::new(io::ErrorKind::InvalidData, "invalid pack index");
        let rest = raw.strip_prefix(IDX_MAGIC).ok_or_else(bad)?;
        if rest.len() < 8 {
            return Err(bad());
        }
        let count = usize::try_from(u64::from_le_bytes(rest[..8].try_into().map_err(|_| bad())?))
            .map_err(|_| bad())?;
        let body = &rest[8..];
        if body.len() != count.checked_mul(IDX_ENTRY).ok_or_else(bad)? {
            return Err(bad());
        }
        let mut ids = Vec::with_capacity(count);
        let mut spans = Vec::with_capacity(count);
        for entry in body.chunks_exact(IDX_ENTRY) {
            ids.push(entry[..32].try_into().map_err(|_| bad())?);
            let offset = u64::from_le_bytes(entry[32..40].try_into().map_err(|_| bad())?);
            let len = u64::from_le_bytes(entry[40..48].try_into().map_err(|_| bad())?);
            spans.push((offset, len));
        }
        if ids.windows(2).any(|w| w[0] >= w[1]) {
            return Err(bad());
        }
        Ok(Self {
            idx_path: idx_path.to_path_buf(),
            pack_path: idx_path.with_extension("hbp"),
            ids,
            spans,
            file: Mutex::new(None),
        })
    }

    fn find(&self, id: &Id) -> Option<(u64, u64)> {
        self.ids.binary_search(id).ok().map(|i| self.spans[i])
    }

    pub fn ids(&self) -> &[Id] {
        &self.ids
    }

    fn read(&self, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let mut guard = self
            .file
            .lock()
            .map_err(|_| io::Error::other("pack lock poisoned"))?;
        if guard.is_none() {
            *guard = Some(File::open(&self.pack_path)?);
        }
        let file = guard
            .as_mut()
            .ok_or_else(|| io::Error::other("pack not open"))?;
        file.seek(SeekFrom::Start(offset))?;
        let len = usize::try_from(len).map_err(|_| io::Error::other("object too large"))?;
        let mut buf = vec![0; len];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// The packs of one object directory. New packs written by other processes
/// are picked up when a lookup misses and the directory has changed.
#[derive(Debug, Default)]
pub(crate) struct PackSet {
    packs: RwLock<Loaded>,
}

#[derive(Debug, Default)]
struct Loaded {
    list: Vec<Arc<Pack>>,
    stamp: Option<SystemTime>,
}

impl PackSet {
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
        for attempt in 0..2 {
            if attempt == 1 {
                self.refresh(dir, false);
            }
            if let Some((pack, offset, len)) = self.lookup(id) {
                match pack.read(offset, len) {
                    Ok(bytes) => return Ok(Some(bytes)),
                    // Removed by `gc` in another process: look again.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        self.refresh(dir, true);
                        if let Some((pack, offset, len)) = self.lookup(id) {
                            return pack.read(offset, len).map(Some);
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
    file: File,
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
            file,
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

    pub fn put(&mut self, id: Id, bytes: &[u8]) -> io::Result<()> {
        if self.entries.contains_key(&id) {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(self.offset))?;
        self.file.write_all(&id)?;
        self.file.write_all(&(bytes.len() as u64).to_le_bytes())?;
        self.file.write_all(bytes)?;
        let start = self.offset + 40;
        let len = bytes.len() as u64;
        self.entries.insert(id, (start, len));
        self.offset = start + len;
        Ok(())
    }

    pub fn get(&mut self, id: &Id) -> io::Result<Option<Vec<u8>>> {
        let Some(&(offset, len)) = self.entries.get(id) else {
            return Ok(None);
        };
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0; usize::try_from(len).map_err(|_| io::Error::other("too large"))?];
        self.file.read_exact(&mut buf)?;
        Ok(Some(buf))
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
        file.sync_all()?;
        drop(file);
        let mut idx = Vec::with_capacity(IDX_MAGIC.len() + 8 + entries.len() * IDX_ENTRY);
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
