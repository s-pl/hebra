//! Reading history: the log, one commit with its diff, and trees and files.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};
use std::collections::{BTreeMap, HashSet, VecDeque};

use hebra_core::blame::StoreHistory;
use hebra_core::diff;
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{COMMIT, Store};
use hebra_core::{ChangeId, ObjectId};

use crate::api::blocking;
use crate::identity::Action;

/// Commits listed when comparing two bookmarks.
const COMPARE_COMMITS: usize = 250;
const SEARCH_MAX_HITS: usize = 200;
const SEARCH_MAX_FILE: usize = 2 * 1024 * 1024;
/// Bytes read per search, so one request cannot read a whole monorepo.
const SEARCH_MAX_BYTES: usize = 512 * 1024 * 1024;

#[derive(Deserialize)]
pub(crate) struct LogQuery {
    rev: Option<String>,
    limit: Option<usize>,
}

pub(crate) fn commit_json(id: &ObjectId, c: &Commit) -> Value {
    json!({
        "commit_id": id.hex(),
        "change_id": c.change_id.encode(),
        "parents": c.parents.iter().map(ObjectId::hex).collect::<Vec<_>>(),
        "description": c.description,
        "author": c.author,
        "timestamp": c.timestamp,
        "authored_at": c.authored_at,
        "provenance": c.provenance,
        "conflicts": c.conflicts,
        "tag": c.tag,
    })
}

pub(crate) async fn log(
    State(s): St,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<LogQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let rev = q.rev.unwrap_or_else(|| "main".to_owned());
    let limit = q.limit.unwrap_or(50).min(500);
    let start = s.resolve(&repo, &rev).await?;
    let bookmarks = s.bookmarks(&repo).await?;
    let store = s.store(&repo);
    let entries = blocking(move || {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([start]);
        let mut commits: Vec<(ObjectId, Commit)> = Vec::new();
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id) || commits.len() >= limit * 4 {
                continue;
            }
            let c: Commit = store.read_json(COMMIT, &id)?;
            if c.change_id == ChangeId::root() && c.parents.is_empty() {
                continue;
            }
            queue.extend(c.parents.iter().copied());
            commits.push((id, c));
        }
        commits.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
        commits.truncate(limit);
        Ok(commits
            .iter()
            .map(|(id, c)| {
                let mut v = commit_json(id, c);
                let labels: Vec<&String> = bookmarks
                    .iter()
                    .filter(|(_, t)| **t == id.hex())
                    .map(|(n, _)| n)
                    .collect();
                v["bookmarks"] = json!(labels);
                v
            })
            .collect::<Vec<Value>>())
    })
    .await?;
    Ok(axum::Json(json!({ "rev": rev, "commits": entries })))
}

pub(crate) async fn commit(
    State(s): St,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let id = s.resolve(&repo, &id).await?;
    let store = s.store(&repo);
    blocking(move || {
        let c: Commit = store.read_json(COMMIT, &id)?;
        let tree: Tree = store.read_tree(&c.tree)?;
        let base = match c.parents.first() {
            Some(p) => {
                let parent: Commit = store.read_json(COMMIT, p)?;
                store.read_tree(&parent.tree)?
            }
            None => Tree::default(),
        };
        let files = diff::diff_trees(&store, &base, &tree, true)?;
        let mut v = commit_json(&id, &c);
        v["files"] = json!(files);
        Ok(axum::Json(v))
    })
    .await
}

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    path: Option<String>,
    /// Return every file path in the tree (for file explorers).
    #[serde(default)]
    recursive: bool,
}

pub(crate) async fn tree(
    State(s): St,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
    headers: HeaderMap,
    Query(q): Query<TreeQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let id = s.resolve(&repo, &id).await?;
    let store = s.store(&repo);
    blocking(move || {
        let c: Commit = store.read_json(COMMIT, &id)?;
        let tree: Tree = store.read_tree(&c.tree)?;
        if q.recursive {
            let files: Vec<&String> = tree.files.keys().collect();
            return Ok(axum::Json(json!({ "kind": "list", "files": files })));
        }
        let path = q.path.unwrap_or_default().trim_matches('/').to_owned();
        if let Some(entry) = tree.files.get(&path) {
            let bytes = store.read(hebra_core::store::BLOB, &entry.blob)?;
            let binary = bytes.contains(&0);
            return Ok(axum::Json(json!({
                "kind": "file",
                "path": path,
                "size": bytes.len(),
                "binary": binary,
                "executable": entry.executable,
                "symlink": entry.symlink,
                "submodule": entry.gitlink,
                "content": if binary { Value::Null } else { Value::String(String::from_utf8_lossy(&bytes).into_owned()) },
            })));
        }
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let mut entries: BTreeMap<String, &str> = BTreeMap::new();
        for (p, entry) in &tree.files {
            if let Some(rest) = p.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((dir, _)) => entries.insert(dir.to_owned(), "dir"),
                    None if entry.symlink => entries.insert(rest.to_owned(), "symlink"),
                    None if entry.gitlink => entries.insert(rest.to_owned(), "submodule"),
                    None => entries.insert(rest.to_owned(), "file"),
                };
            }
        }
        if entries.is_empty() && !path.is_empty() {
            return Err(ApiError::not_found(format!("path `{path}` not found")));
        }
        let mut list: Vec<Value> = entries
            .into_iter()
            .map(|(name, kind)| json!({ "name": name, "kind": kind }))
            .collect();
        list.sort_by_key(|e| {
            (
                e["kind"] != "dir",
                e["name"].as_str().unwrap_or_default().to_owned(),
            )
        });
        Ok(axum::Json(json!({ "kind": "dir", "path": path, "entries": list })))
    })
    .await
}

#[derive(Deserialize)]
pub(crate) struct BlameQuery {
    path: String,
}

/// Who wrote each line of a file.
pub(crate) async fn blame(
    State(s): St,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
    headers: HeaderMap,
    Query(q): Query<BlameQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let id = s.resolve(&repo, &id).await?;
    let store = s.store(&repo);
    blocking(move || {
        let lines = hebra_core::blame::blame(&mut StoreHistory::new(&store), id, &q.path)?;
        let mut commits: BTreeMap<String, Value> = BTreeMap::new();
        for line in &lines {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                commits.entry(line.commit.hex())
            {
                let c: Commit = store.read_json(COMMIT, &line.commit)?;
                slot.insert(commit_json(&line.commit, &c));
            }
        }
        Ok(axum::Json(json!({
            "path": q.path,
            "lines": lines,
            "commits": commits,
        })))
    })
    .await
}

#[derive(Deserialize)]
pub(crate) struct CompareQuery {
    base: String,
    head: String,
}

/// What `head` adds to `base`: its commits, and the diff since they parted.
pub(crate) async fn compare(
    State(s): St,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<CompareQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let base = s.resolve(&repo, &q.base).await?;
    let head = s.resolve(&repo, &q.head).await?;
    let store = s.store(&repo);
    blocking(move || {
        let fork = crate::merging::merge_base(&store, base, head)?;
        let from = match fork {
            Some(id) => {
                let c: Commit = store.read_json(COMMIT, &id)?;
                store.read_tree(&c.tree)?
            }
            None => Tree::default(),
        };
        let tip: Commit = store.read_json(COMMIT, &head)?;
        let files = diff::diff_trees(&store, &from, &store.read_tree(&tip.tree)?, true)?;
        let behind = ancestor_set(&store, base)?;
        let mut commits = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([head]);
        while let Some(id) = queue.pop_front() {
            if behind.contains(&id) || !seen.insert(id) || commits.len() >= COMPARE_COMMITS {
                continue;
            }
            let c: Commit = store.read_json(COMMIT, &id)?;
            queue.extend(c.parents.iter().copied());
            commits.push(commit_json(&id, &c));
        }
        Ok(axum::Json(json!({
            "base": base.hex(),
            "head": head.hex(),
            "fork": fork.map(|id| id.hex()),
            "commits": commits,
            "files": files,
        })))
    })
    .await
}

fn ancestor_set(store: &Store, start: ObjectId) -> hebra_core::Result<HashSet<ObjectId>> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        if seen.insert(id) {
            let c: Commit = store.read_json(COMMIT, &id)?;
            queue.extend(c.parents);
        }
    }
    Ok(seen)
}

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    q: String,
    rev: Option<String>,
}

/// Finds text in the files of a revision. Matching is plain and ignores
/// case; results are capped, and files too large to be source are skipped.
pub(crate) async fn search(
    State(s): St,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let needle = q.q.trim().to_lowercase();
    if needle.is_empty() || needle.len() > 256 {
        return Err(ApiError::bad("search for 1 to 256 characters"));
    }
    let rev = q.rev.unwrap_or_else(|| "main".to_owned());
    let id = s.resolve(&repo, &rev).await?;
    let store = s.store(&repo);
    blocking(move || {
        let c: Commit = store.read_json(COMMIT, &id)?;
        let tree = store.read_tree(&c.tree)?;
        let mut hits = Vec::new();
        let mut files = 0;
        let mut scanned = 0;
        let mut partial = false;
        'files: for (path, entry) in &tree.files {
            if scanned > SEARCH_MAX_BYTES {
                partial = true;
                break;
            }
            if !entry.is_file() {
                continue;
            }
            let path_matches = path.to_lowercase().contains(&needle);
            let data = store.read(hebra_core::store::BLOB, &entry.blob)?;
            scanned += data.len();
            if data.len() > SEARCH_MAX_FILE || data.contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&data);
            let mut found = false;
            for (n, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    found = true;
                    hits.push(json!({ "path": path, "line": n + 1, "text": line }));
                    if hits.len() >= SEARCH_MAX_HITS {
                        files += 1;
                        break 'files;
                    }
                }
            }
            if found || path_matches {
                files += 1;
            }
            if path_matches && !found {
                hits.push(json!({ "path": path, "line": 0, "text": "" }));
            }
        }
        Ok(axum::Json(json!({
            "rev": rev,
            "files": files,
            "matches": hits,
            "truncated": partial || hits.len() >= SEARCH_MAX_HITS,
        })))
    })
    .await
}
