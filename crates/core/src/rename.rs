//! Rename detection. Like Git, Hebra does not record renames: it works them
//! out when it matters, by comparing what disappeared with what appeared.

use std::collections::{BTreeMap, HashMap};

use crate::error::Result;
use crate::model::{FileEntry, Tree};
use crate::store::{BLOB, Store};
use crate::wc::is_binary;

/// How much of a file has to stay the same to call it a rename, in percent
/// (Git's default too).
pub const MIN_SIMILARITY: u8 = 50;
/// Above this many deleted×added pairs only exact matches are looked for
/// (Git's `diff.renameLimit` plays the same role).
const PAIR_LIMIT: usize = 1_000_000;
/// Files larger than this are only matched exactly.
const MAX_COMPARE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Rename {
    pub from: String,
    pub to: String,
    pub similarity: u8,
}

fn line_hashes(data: &[u8]) -> HashMap<u64, u32> {
    let mut counts = HashMap::new();
    for line in data.split(|&b| b == b'\n') {
        let digest = blake3::hash(line);
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
        *counts.entry(u64::from_le_bytes(bytes)).or_insert(0) += 1;
    }
    counts
}

/// Percent of lines two texts share (each line counted as often as it
/// appears on both sides).
#[must_use]
pub fn similarity(a: &[u8], b: &[u8]) -> u8 {
    if a == b {
        return 100;
    }
    let (x, y) = (line_hashes(a), line_hashes(b));
    let total_x: u64 = x.values().map(|n| u64::from(*n)).sum();
    let total_y: u64 = y.values().map(|n| u64::from(*n)).sum();
    let common: u64 = x
        .iter()
        .map(|(line, n)| u64::from((*n).min(y.get(line).copied().unwrap_or(0))))
        .sum();
    let total = total_x.max(total_y).max(1);
    u8::try_from(common * 100 / total).unwrap_or(100)
}

/// Files that left `from` and arrived in `to` under another name. Each file
/// is used once; exact matches go first, then the most similar.
pub fn detect(store: &Store, from: &Tree, to: &Tree) -> Result<Vec<Rename>> {
    let is_plain = |e: &FileEntry| e.is_file();
    let mut deleted: BTreeMap<&String, &FileEntry> = from
        .files
        .iter()
        .filter(|(p, e)| !to.files.contains_key(*p) && is_plain(e))
        .collect();
    let mut added: BTreeMap<&String, &FileEntry> = to
        .files
        .iter()
        .filter(|(p, e)| !from.files.contains_key(*p) && is_plain(e))
        .collect();
    let mut renames = Vec::new();
    if deleted.is_empty() || added.is_empty() {
        return Ok(renames);
    }

    let mut by_blob: HashMap<_, Vec<&String>> = HashMap::new();
    for (path, entry) in &added {
        by_blob.entry(entry.blob).or_default().push(path);
    }
    for (old, entry) in deleted.clone() {
        if let Some(new) = by_blob.get_mut(&entry.blob).and_then(Vec::pop) {
            renames.push(Rename {
                from: old.clone(),
                to: new.clone(),
                similarity: 100,
            });
            deleted.remove(old);
            added.remove(new);
        }
    }
    if deleted.is_empty()
        || added.is_empty()
        || deleted.len().saturating_mul(added.len()) > PAIR_LIMIT
    {
        return Ok(renames);
    }

    let read = |entry: &FileEntry| -> Result<Option<Vec<u8>>> {
        let data = store.read(BLOB, &entry.blob)?;
        Ok((!is_binary(&data) && data.len() <= MAX_COMPARE).then_some(data))
    };
    let mut new_files = Vec::new();
    for (path, entry) in &added {
        if let Some(data) = read(entry)? {
            new_files.push((*path, data));
        }
    }
    let mut scored = Vec::new();
    for (old, entry) in &deleted {
        let Some(data) = read(entry)? else { continue };
        for (new, new_data) in &new_files {
            // Very different sizes cannot reach the threshold.
            let (small, large) = (
                data.len().min(new_data.len()),
                data.len().max(new_data.len()),
            );
            if small * 100 < large * usize::from(MIN_SIMILARITY) {
                continue;
            }
            let score = similarity(&data, new_data);
            if score >= MIN_SIMILARITY {
                scored.push((score, *old, *new));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)).then(a.2.cmp(b.2)));
    for (score, old, new) in scored {
        if deleted.remove(old).is_some() {
            if added.remove(new).is_some() {
                renames.push(Rename {
                    from: old.clone(),
                    to: new.clone(),
                    similarity: score,
                });
            } else {
                // The new name is taken: `old` may still match another one.
                deleted.insert(old, &from.files[old]);
            }
        }
    }
    renames.sort_by(|a, b| a.to.cmp(&b.to));
    Ok(renames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(store: &Store, files: &[(&str, &str)]) -> Tree {
        let mut tree = Tree::default();
        for (path, content) in files {
            let blob = store.write(BLOB, content.as_bytes()).unwrap();
            tree.files
                .insert((*path).to_owned(), FileEntry::file(blob, false));
        }
        tree
    }

    #[test]
    fn finds_exact_and_similar_renames() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let edited = format!("{body}one more\n");
        let a = tree(
            &store,
            &[
                ("same.txt", "unchanged\n"),
                ("old.rs", &body),
                ("x.txt", "exact\n"),
            ],
        );
        let b = tree(
            &store,
            &[
                ("same.txt", "unchanged\n"),
                ("new.rs", &edited),
                ("y.txt", "exact\n"),
            ],
        );
        let found = detect(&store, &a, &b).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].from.as_str(), found[0].to.as_str()),
            ("old.rs", "new.rs")
        );
        assert!(found[0].similarity >= 90 && found[0].similarity < 100);
        assert_eq!(found[1].similarity, 100);
    }

    #[test]
    fn unrelated_files_are_not_renames() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let a = tree(&store, &[("a.txt", "one\ntwo\nthree\n")]);
        let b = tree(&store, &[("b.txt", "completely\ndifferent\ncontent\n")]);
        assert!(detect(&store, &a, &b).unwrap().is_empty());
    }
}
