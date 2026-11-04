//! Reading a Git object database directly: pack files (through their
//! version 2 `.idx`) and loose objects, decoded on every core.
//!
//! `git fast-export` turns history into whole files and per-commit changes,
//! which then cost as much again to turn back into trees. Reading the
//! objects instead lets an import convert each Git tree once and reuse the
//! deltas Git already chose.
//!
//! Deltas are resolved the way `git index-pack` does it: every object that
//! is not a delta starts a task, and each decoded object hands its bytes to
//! the objects stored as deltas against it. Each object is inflated once.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use crate::error::{Error, Result};

/// A Git object id (SHA-1).
pub type Sha = [u8; 20];

/// Packs larger than this together are not read into memory.
const MAX_PACK_BYTES: u64 = 4 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Commit,
    Tree,
    Blob,
    Tag,
}

impl Kind {
    const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Commit,
            2 => Self::Tree,
            3 => Self::Blob,
            4 => Self::Tag,
            _ => return None,
        })
    }

    fn from_name(name: &[u8]) -> Option<Self> {
        Some(match name {
            b"commit" => Self::Commit,
            b"tree" => Self::Tree,
            b"blob" => Self::Blob,
            b"tag" => Self::Tag,
            _ => return None,
        })
    }
}

/// One decoded object.
pub struct Object<'a> {
    pub sha: Sha,
    pub kind: Kind,
    pub data: &'a [u8],
    /// When Git stored it as a delta: the base object, its bytes and the
    /// instructions (Git's delta format, see [`for_each_git_op`]).
    pub delta: Option<Delta<'a>>,
}

#[derive(Clone, Copy)]
pub struct Delta<'a> {
    pub base: Sha,
    pub base_data: &'a [u8],
    pub instructions: &'a [u8],
}

fn unsupported(why: impl Into<String>) -> Error {
    Error::InvalidArgument(format!(
        "cannot read the Git objects directly: {}",
        why.into()
    ))
}

#[must_use]
pub fn sha_hex(sha: &Sha) -> String {
    hex::encode(sha)
}

pub fn parse_sha(text: &str) -> Option<Sha> {
    hex::decode(text.trim()).ok()?.try_into().ok()
}

#[derive(Debug, Clone, Copy)]
enum Base {
    None,
    Offset(u64),
    Sha(Sha),
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    sha: Sha,
    offset: u64,
    /// Start of the compressed data.
    data: usize,
    /// End of the compressed data (the next object, or the trailer).
    end: usize,
    code: u8,
    size: usize,
    base: Base,
}

struct Pack {
    bytes: Vec<u8>,
    /// Sorted by offset.
    entries: Vec<Entry>,
    /// Objects stored as deltas against each entry.
    children: Vec<Vec<u32>>,
    roots: Vec<u32>,
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// `(sha, offset)` of every object in a version 2 index.
fn read_idx(idx: &[u8]) -> Option<Vec<(Sha, u64)>> {
    if idx.get(..8)? != b"\xfftOc\0\0\0\x02" {
        return None;
    }
    let count = be32(idx, 8 + 255 * 4)? as usize;
    let shas = 8 + 256 * 4;
    let crcs = shas + count * 20;
    let small = crcs + count * 4;
    let large = small + count * 4;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let sha: Sha = idx
            .get(shas + i * 20..shas + (i + 1) * 20)?
            .try_into()
            .ok()?;
        let off = be32(idx, small + i * 4)?;
        let offset = if off & 0x8000_0000 == 0 {
            u64::from(off)
        } else {
            be64(idx, large + (off & 0x7fff_ffff) as usize * 8)?
        };
        out.push((sha, offset));
    }
    Some(out)
}

impl Pack {
    fn open(pack_path: &Path) -> Result<Self> {
        let io = |source| Error::Io {
            path: pack_path.to_path_buf(),
            source,
        };
        let idx = fs::read(pack_path.with_extension("idx")).map_err(io)?;
        let mut listed = read_idx(&idx).ok_or_else(|| unsupported("unknown pack index format"))?;
        let bytes = fs::read(pack_path).map_err(io)?;
        if bytes.get(..4) != Some(b"PACK") || bytes.len() < 32 {
            return Err(unsupported("not a pack file"));
        }
        listed.sort_by_key(|(_, offset)| *offset);
        let trailer = bytes.len() - 20;
        let mut entries = Vec::with_capacity(listed.len());
        for (i, (sha, offset)) in listed.iter().enumerate() {
            let end = listed.get(i + 1).map_or(trailer as u64, |(_, o)| *o);
            let start = usize::try_from(*offset).map_err(|_| unsupported("offset"))?;
            let end = usize::try_from(end).map_err(|_| unsupported("offset"))?;
            entries.push(parse_header(&bytes, *sha, start, end)?);
        }
        let by_sha: HashMap<Sha, u32> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.sha, i as u32))
            .collect();
        let mut children = vec![Vec::new(); entries.len()];
        let mut roots = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let base = match e.base {
                Base::None => {
                    roots.push(i as u32);
                    continue;
                }
                Base::Offset(at) => entries
                    .binary_search_by_key(&at, |x| x.offset)
                    .ok()
                    .map(|b| b as u32),
                Base::Sha(sha) => by_sha.get(&sha).copied(),
            };
            let base = base.ok_or_else(|| unsupported("a delta base is outside its pack"))?;
            children[base as usize].push(i as u32);
        }
        Ok(Self {
            bytes,
            entries,
            children,
            roots,
        })
    }
}

fn parse_header(bytes: &[u8], sha: Sha, start: usize, end: usize) -> Result<Entry> {
    let bad = || unsupported("corrupt pack entry");
    let mut at = start;
    let mut byte = *bytes.get(at).ok_or_else(bad)?;
    at += 1;
    let code = (byte >> 4) & 7;
    let mut size = usize::from(byte & 15);
    let mut shift = 4;
    while byte & 0x80 != 0 {
        byte = *bytes.get(at).ok_or_else(bad)?;
        at += 1;
        size |= usize::from(byte & 0x7f)
            .checked_shl(shift)
            .ok_or_else(bad)?;
        shift += 7;
        if shift > 60 {
            return Err(bad());
        }
    }
    let base = match code {
        6 => {
            let mut byte = *bytes.get(at).ok_or_else(bad)?;
            at += 1;
            let mut back = u64::from(byte & 0x7f);
            while byte & 0x80 != 0 {
                byte = *bytes.get(at).ok_or_else(bad)?;
                at += 1;
                back = ((back + 1) << 7) | u64::from(byte & 0x7f);
            }
            Base::Offset((start as u64).checked_sub(back).ok_or_else(bad)?)
        }
        7 => {
            let base: Sha = bytes
                .get(at..at + 20)
                .ok_or_else(bad)?
                .try_into()
                .map_err(|_| bad())?;
            at += 20;
            Base::Sha(base)
        }
        1..=4 => Base::None,
        _ => return Err(bad()),
    };
    if at > end {
        return Err(bad());
    }
    Ok(Entry {
        sha,
        offset: start as u64,
        data: at,
        end,
        code,
        size,
        base,
    })
}

fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>> {
    let out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, size.max(1))
        .map_err(|_| unsupported("corrupt compressed object"))?;
    if out.len() != size {
        return Err(unsupported("object size does not match"));
    }
    Ok(out)
}

fn varint(data: &[u8], at: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*at)?;
        *at += 1;
        value |= usize::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= usize::BITS {
            return None;
        }
    }
}

/// One instruction of a Git delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOp<'a> {
    Copy { offset: usize, len: usize },
    Insert(&'a [u8]),
}

/// Walks a Git delta: returns `(base size, result size)` after calling `op`
/// for each instruction, or `None` if it is corrupt.
pub fn for_each_git_op<'a>(
    delta: &'a [u8],
    op: &mut dyn FnMut(GitOp<'a>),
) -> Option<(usize, usize)> {
    let mut at = 0;
    let base_size = varint(delta, &mut at)?;
    let size = varint(delta, &mut at)?;
    while at < delta.len() {
        let cmd = delta[at];
        at += 1;
        if cmd & 0x80 != 0 {
            let mut offset = 0usize;
            let mut len = 0usize;
            for bit in 0..4 {
                if cmd & (1 << bit) != 0 {
                    offset |= usize::from(*delta.get(at)?) << (8 * bit);
                    at += 1;
                }
            }
            for bit in 0..3 {
                if cmd & (0x10 << bit) != 0 {
                    len |= usize::from(*delta.get(at)?) << (8 * bit);
                    at += 1;
                }
            }
            if len == 0 {
                len = 0x10000;
            }
            op(GitOp::Copy { offset, len });
        } else if cmd != 0 {
            let len = usize::from(cmd);
            op(GitOp::Insert(delta.get(at..at + len)?));
            at += len;
        } else {
            return None;
        }
    }
    Some((base_size, size))
}

fn apply_git_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>> {
    let bad = || unsupported("corrupt delta");
    let mut out = Vec::new();
    let mut ok = true;
    let (base_size, size) = for_each_git_op(delta, &mut |op| match op {
        GitOp::Copy { offset, len } => {
            match offset.checked_add(len).and_then(|e| base.get(offset..e)) {
                Some(part) => out.extend_from_slice(part),
                None => ok = false,
            }
        }
        GitOp::Insert(bytes) => out.extend_from_slice(bytes),
    })
    .ok_or_else(bad)?;
    if !ok || base_size != base.len() || size != out.len() {
        return Err(bad());
    }
    Ok(out)
}

enum Task {
    Root {
        pack: u32,
        entry: u32,
    },
    Child {
        pack: u32,
        entry: u32,
        base: Arc<Vec<u8>>,
        base_sha: Sha,
        kind: Kind,
    },
    Loose(PathBuf, Sha),
}

#[derive(Default)]
struct Queue {
    tasks: Vec<Task>,
    active: usize,
    failed: Option<Error>,
}

/// The pack files of a Git object directory.
fn pack_paths(objects: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(objects.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pack") && p.with_extension("idx").is_file())
        .collect();
    out.sort();
    out
}

fn loose_objects(objects: &Path) -> Vec<(PathBuf, Sha)> {
    let mut out = Vec::new();
    for dir in fs::read_dir(objects).into_iter().flatten().flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        if name.len() != 2 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        for file in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let rest = file.file_name().to_string_lossy().into_owned();
            if let Some(sha) = parse_sha(&format!("{name}{rest}")) {
                out.push((file.path(), sha));
            }
        }
    }
    out
}

/// Whether a Git directory can be read here: packs with version 2
/// indexes, no alternates, not too large.
#[must_use]
pub fn readable(git_dir: &Path) -> bool {
    let objects = git_dir.join("objects");
    if objects.join("info").join("alternates").is_file() {
        return false;
    }
    let total: u64 = pack_paths(&objects)
        .iter()
        .filter_map(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    total <= MAX_PACK_BYTES
}

/// Decodes every object of `git_dir` (a bare repository or a `.git`
/// directory) and calls `visit` with each, from several threads at once.
/// A delta's base is always visited before the delta.
pub fn for_each_object(
    git_dir: &Path,
    visit: &(dyn Fn(Object<'_>) -> Result<()> + Sync),
) -> Result<()> {
    let objects = git_dir.join("objects");
    if !readable(git_dir) {
        return Err(unsupported("alternates or very large packs"));
    }
    let packs: Vec<Pack> = pack_paths(&objects)
        .iter()
        .map(|p| Pack::open(p))
        .collect::<Result<_>>()?;
    let mut queue = Queue::default();
    for (p, pack) in packs.iter().enumerate() {
        queue
            .tasks
            .extend(pack.roots.iter().map(|&entry| Task::Root {
                pack: p as u32,
                entry,
            }));
    }
    queue.tasks.extend(
        loose_objects(&objects)
            .into_iter()
            .map(|(path, sha)| Task::Loose(path, sha)),
    );
    let queue = Mutex::new(queue);
    let wake = Condvar::new();
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(16));

    let work = || {
        loop {
            let task = {
                let Ok(mut q) = queue.lock() else { return };
                loop {
                    if q.failed.is_some() {
                        return;
                    }
                    if let Some(task) = q.tasks.pop() {
                        q.active += 1;
                        break task;
                    }
                    if q.active == 0 {
                        return;
                    }
                    q = match wake.wait(q) {
                        Ok(q) => q,
                        Err(_) => return,
                    };
                }
            };
            let result = run(&packs, task, visit);
            let Ok(mut q) = queue.lock() else { return };
            q.active -= 1;
            match result {
                Ok(children) => {
                    let n = children.len();
                    q.tasks.extend(children);
                    if n > 0 || (q.tasks.is_empty() && q.active == 0) {
                        wake.notify_all();
                    }
                }
                Err(e) => {
                    q.failed.get_or_insert(e);
                    wake.notify_all();
                }
            }
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..workers {
            scope.spawn(work);
        }
        work();
    });
    let queue = queue
        .into_inner()
        .map_err(|_| unsupported("a worker panicked"))?;
    match queue.failed {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn run(
    packs: &[Pack],
    task: Task,
    visit: &(dyn Fn(Object<'_>) -> Result<()> + Sync),
) -> Result<Vec<Task>> {
    let (p, i, kind, data) = match task {
        Task::Loose(path, sha) => {
            let file = fs::read(&path).map_err(|source| Error::Io { path, source })?;
            let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&file)
                .map_err(|_| unsupported("corrupt loose object"))?;
            let nul = raw
                .iter()
                .position(|&b| b == 0)
                .ok_or_else(|| unsupported("loose object header"))?;
            let kind = raw[..nul]
                .split(|&b| b == b' ')
                .next()
                .and_then(Kind::from_name)
                .ok_or_else(|| unsupported("loose object kind"))?;
            visit(Object {
                sha,
                kind,
                data: &raw[nul + 1..],
                delta: None,
            })?;
            return Ok(Vec::new());
        }
        Task::Root { pack, entry } => {
            let e = &packs[pack as usize].entries[entry as usize];
            let bytes = &packs[pack as usize].bytes;
            let data = inflate(&bytes[e.data..e.end], e.size)?;
            let kind = Kind::from_code(e.code).ok_or_else(|| unsupported("object kind"))?;
            visit(Object {
                sha: e.sha,
                kind,
                data: &data,
                delta: None,
            })?;
            (pack, entry, kind, data)
        }
        Task::Child {
            pack,
            entry,
            base,
            base_sha,
            kind,
        } => {
            let e = &packs[pack as usize].entries[entry as usize];
            let bytes = &packs[pack as usize].bytes;
            let delta = inflate(&bytes[e.data..e.end], e.size)?;
            let data = apply_git_delta(&base, &delta)?;
            visit(Object {
                sha: e.sha,
                kind,
                data: &data,
                delta: Some(Delta {
                    base: base_sha,
                    base_data: &base,
                    instructions: &delta,
                }),
            })?;
            drop(base);
            (pack, entry, kind, data)
        }
    };
    let pack = &packs[p as usize];
    let children = &pack.children[i as usize];
    if children.is_empty() {
        return Ok(Vec::new());
    }
    let sha = pack.entries[i as usize].sha;
    let shared = Arc::new(data);
    Ok(children
        .iter()
        .map(|&entry| Task::Child {
            pack: p,
            entry,
            base: Arc::clone(&shared),
            base_sha: sha,
            kind,
        })
        .collect())
}

/// One entry of a Git tree object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeEntry<'a> {
    pub mode: u32,
    pub name: &'a [u8],
    pub sha: Sha,
}

/// The entries of a Git tree object, or `None` if it is corrupt.
#[must_use]
pub fn tree_entries(data: &[u8]) -> Option<Vec<TreeEntry<'_>>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let space = at + data[at..].iter().position(|&b| b == b' ')?;
        let mode = u32::from_str_radix(std::str::from_utf8(&data[at..space]).ok()?, 8).ok()?;
        let nul = space + 1 + data[space + 1..].iter().position(|&b| b == 0)?;
        let name = &data[space + 1..nul];
        let sha: Sha = data.get(nul + 1..nul + 21)?.try_into().ok()?;
        out.push(TreeEntry { mode, name, sha });
        at = nul + 21;
    }
    Some(out)
}

/// A commit or tag object: its headers (continuation lines joined with
/// `\n`) and the message after the blank line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headers<'a> {
    pub fields: Vec<(&'a [u8], Vec<u8>)>,
    pub message: &'a [u8],
}

impl Headers<'_> {
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_slice())
    }

    pub fn all<'s>(&'s self, key: &'s [u8]) -> impl Iterator<Item = &'s [u8]> + 's {
        self.fields
            .iter()
            .filter(move |(k, _)| *k == key)
            .map(|(_, v)| v.as_slice())
    }
}

#[must_use]
pub fn headers(data: &[u8]) -> Headers<'_> {
    let mut fields: Vec<(&[u8], Vec<u8>)> = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let end = data[at..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(data.len(), |n| at + n);
        let line = &data[at..end];
        at = (end + 1).min(data.len());
        if line.is_empty() {
            return Headers {
                fields,
                message: &data[at..],
            };
        }
        if let Some(rest) = line.strip_prefix(b" ") {
            if let Some((_, value)) = fields.last_mut() {
                value.push(b'\n');
                value.extend_from_slice(rest);
            }
            continue;
        }
        let (key, value) = match line.iter().position(|&b| b == b' ') {
            Some(space) => (&line[..space], &line[space + 1..]),
            None => (line, &line[line.len()..]),
        };
        fields.push((key, value.to_vec()));
    }
    Headers {
        fields,
        message: &data[data.len()..],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_deltas_apply() {
        let base = b"hello world, this is the base text";
        // base size 34, result size 11: copy 0..6, insert "there".
        let delta = [34, 11, 0x90, 6, 5, b't', b'h', b'e', b'r', b'e'];
        assert_eq!(apply_git_delta(base, &delta).unwrap(), b"hello there");
        assert!(apply_git_delta(b"short", &delta).is_err());
    }

    #[test]
    fn trees_and_headers_parse() {
        let mut tree = b"100644 a.txt\0".to_vec();
        tree.extend_from_slice(&[1; 20]);
        tree.extend_from_slice(b"40000 src\0");
        tree.extend_from_slice(&[2; 20]);
        let entries = tree_entries(&tree).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].mode, 0o100_644);
        assert_eq!(entries[1].name, b"src");
        assert_eq!(entries[1].mode, 0o40000);

        let commit = b"tree abc\nparent p1\nparent p2\ngpgsig -----BEGIN\n line\n -----END\nauthor A <a> 1 +0000\n\nmessage\n";
        let h = headers(commit);
        assert_eq!(h.get(b"tree"), Some(&b"abc"[..]));
        assert_eq!(h.all(b"parent").count(), 2);
        assert_eq!(h.get(b"gpgsig"), Some(&b"-----BEGIN\nline\n-----END"[..]));
        assert_eq!(h.message, b"message\n");
    }
}
