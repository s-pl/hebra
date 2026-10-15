//! Moving objects between repositories: raw object access, a simple binary
//! pack format, and the set of objects a push or pull has to send.
//!
//! Pack format: the magic line `HBPK1\n`, then for each object its 32-byte
//! id, its length as a little-endian `u64`, and its raw bytes
//! (`<kind>\0<data>`, exactly as stored on disk). Every object is verified
//! against its id when the pack is decoded.

use std::collections::{HashSet, VecDeque};

use crate::error::{Error, Result};
use crate::ids::ObjectId;
use crate::model::{Commit, Tree};
use crate::store::{BLOB, COMMIT, Store, TREE};

const MAGIC: &[u8] = b"HBPK1\n";
/// Refuse absurd object sizes in a pack (protects servers from bad input).
const MAX_OBJECT: u64 = 512 * 1024 * 1024;

/// The kind of a raw object (`blob`, `tree` or `commit`).
#[must_use]
pub fn kind_of(raw: &[u8]) -> Option<&'static str> {
    let nul = raw.iter().position(|&b| b == 0)?;
    match &raw[..nul] {
        b"blob" => Some(BLOB),
        b"tree" => Some(TREE),
        b"commit" => Some(COMMIT),
        _ => None,
    }
}

#[must_use]
pub fn encode(objects: &[(ObjectId, Vec<u8>)]) -> Vec<u8> {
    let size = MAGIC.len() + objects.iter().map(|(_, raw)| 40 + raw.len()).sum::<usize>();
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(MAGIC);
    for (id, raw) in objects {
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        out.extend_from_slice(raw);
    }
    out
}

pub fn decode(pack: &[u8]) -> Result<Vec<(ObjectId, Vec<u8>)>> {
    let bad = |why: &str| Error::InvalidArgument(format!("invalid pack: {why}"));
    let mut rest = pack
        .strip_prefix(MAGIC)
        .ok_or_else(|| bad("missing header"))?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.len() < 40 {
            return Err(bad("truncated entry"));
        }
        let id: [u8; 32] = rest[..32].try_into().map_err(|_| bad("id"))?;
        let len_bytes: [u8; 8] = rest[32..40].try_into().map_err(|_| bad("length"))?;
        let len = u64::from_le_bytes(len_bytes);
        if len > MAX_OBJECT {
            return Err(bad("object too large"));
        }
        let len = usize::try_from(len).map_err(|_| bad("length"))?;
        rest = &rest[40..];
        if rest.len() < len {
            return Err(bad("truncated object"));
        }
        let raw = rest[..len].to_vec();
        rest = &rest[len..];
        let actual = blake3::hash(&raw);
        if actual.as_bytes() != &id || kind_of(&raw).is_none() {
            return Err(Error::CorruptObject(hex::encode(id)));
        }
        out.push((ObjectId::parse(&hex::encode(id))?, raw));
    }
    Ok(out)
}

/// Commits reachable from `wants`, stopping at commits in `haves` (whose
/// ancestry the other side already has), plus every tree and blob they use.
pub fn closure(
    store: &Store,
    wants: &[ObjectId],
    haves: &HashSet<ObjectId>,
) -> Result<Vec<ObjectId>> {
    let mut objects = Vec::new();
    let mut seen = HashSet::new();
    let mut queue: VecDeque<ObjectId> = wants.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        if haves.contains(&id) || !seen.insert(id) {
            continue;
        }
        let commit: Commit = store.read_json(COMMIT, &id)?;
        objects.push(id);
        if seen.insert(commit.tree) {
            objects.push(commit.tree);
            let tree: Tree = store.read_json(TREE, &commit.tree)?;
            for entry in tree.files.values() {
                if seen.insert(entry.blob) {
                    objects.push(entry.blob);
                }
            }
        }
        queue.extend(commit.parents);
    }
    Ok(objects)
}

/// Whether a commit and everything one level down (tree, blobs, parents) is
/// present, which a server checks before moving a bookmark onto it.
pub fn is_complete(store: &Store, id: &ObjectId) -> Result<bool> {
    if !store.has(id) {
        return Ok(false);
    }
    let commit: Commit = store.read_json(COMMIT, id)?;
    if !store.has(&commit.tree) || !commit.parents.iter().all(|p| store.has(p)) {
        return Ok(false);
    }
    let tree: Tree = store.read_json(TREE, &commit.tree)?;
    Ok(tree.files.values().all(|e| store.has(&e.blob)))
}

/// Reads objects for a pack.
pub fn collect(store: &Store, ids: &[ObjectId]) -> Result<Vec<(ObjectId, Vec<u8>)>> {
    ids.iter()
        .map(|id| Ok((*id, store.read_raw(id)?)))
        .collect()
}

/// Stores every object of a decoded pack; returns how many were new.
pub fn store_all(store: &Store, objects: &[(ObjectId, Vec<u8>)]) -> Result<usize> {
    let mut new = 0;
    for (id, raw) in objects {
        if !store.has(id) {
            store.write_raw(raw)?;
            new += 1;
        }
    }
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trips_and_rejects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let id = store.write(BLOB, b"hello").unwrap();
        let pack = encode(&collect(&store, &[id]).unwrap());
        let decoded = decode(&pack).unwrap();
        assert_eq!(decoded[0].0, id);

        let mut tampered = pack.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(decode(&tampered).is_err());
        assert!(decode(b"nope").is_err());
    }
}
