//! Reading history: the log, one commit with its diff, and trees and files.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};
use std::collections::{BTreeMap, HashSet, VecDeque};

use hebra_core::diff;
use hebra_core::model::{Commit, Tree};
use hebra_core::store::COMMIT;
use hebra_core::{ChangeId, ObjectId};

use crate::api::blocking;
use crate::identity::Action;

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
        "provenance": c.provenance,
        "conflicts": c.conflicts,
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
