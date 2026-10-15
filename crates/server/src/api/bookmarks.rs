//! Moving bookmarks: compare-and-swap, authorized per bookmark, and checked
//! against the repository policy (`.hb/policy.toml`).

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{COMMIT, Store, TREE};
use hebra_core::{ObjectId, transport};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{St, blocking};
use crate::checks;
use crate::db::Repo;
use crate::error::{ApiError, ApiResult};
use crate::identity::{Action, Principal};
use crate::names::{glob_match, valid_bookmark};
use crate::server::Server;

#[derive(Deserialize)]
pub(crate) struct UpdateBookmark {
    name: String,
    old: Option<String>,
    new: Option<String>,
}

fn tree_of(store: &Store, commit: Option<ObjectId>) -> ApiResult<Tree> {
    Ok(match commit {
        Some(id) => {
            let c: Commit = store.read_json(COMMIT, &id)?;
            store.read_json(TREE, &c.tree)?
        }
        None => Tree::default(),
    })
}

/// Paths that differ between two commits.
pub(crate) fn changed_paths(
    store: &Store,
    from: Option<ObjectId>,
    to: Option<ObjectId>,
) -> ApiResult<Vec<String>> {
    let (a, b) = (tree_of(store, from)?, tree_of(store, to)?);
    let mut paths: Vec<String> = a
        .files
        .iter()
        .filter(|(p, e)| {
            b.files
                .get(*p)
                .is_none_or(|f| f.blob != e.blob || f.executable != e.executable)
        })
        .map(|(p, _)| p.clone())
        .collect();
    paths.extend(
        b.files
            .keys()
            .filter(|p| !a.files.contains_key(*p))
            .cloned(),
    );
    paths.sort();
    Ok(paths)
}

/// Refuses changes to files the policy keeps away from agents.
pub(crate) async fn check_agent_paths(
    s: &Server,
    repo: &Repo,
    who: &Principal,
    policy: &checks::Policy,
    from: Option<ObjectId>,
    to: Option<ObjectId>,
) -> ApiResult<()> {
    let denied: Vec<String> = policy
        .paths
        .iter()
        .filter(|p| p.agents.as_deref() == Some("deny"))
        .map(|p| p.pattern.clone())
        .collect();
    if denied.is_empty() || !who.is_agent() {
        return Ok(());
    }
    let store = s.store(repo);
    let changed = blocking(move || changed_paths(&store, from, to)).await?;
    if let Some(path) = changed
        .iter()
        .find(|p| denied.iter().any(|d| glob_match(d, p)))
    {
        return Err(ApiError::forbidden(
            "policy",
            format!("agents may not change `{path}` (.hb/policy.toml); ask a person"),
        ));
    }
    Ok(())
}

pub(crate) async fn update(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<UpdateBookmark>,
) -> ApiResult<axum::Json<Value>> {
    if !valid_bookmark(&body.name) {
        return Err(ApiError::bad(format!(
            "invalid bookmark name `{}`",
            body.name
        )));
    }
    let (repo, who) = s
        .access(&headers, &owner, &name, Action::Push(&body.name))
        .await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    let new = body.new.as_deref().map(ObjectId::parse).transpose()?;
    let old = body.old.as_deref().map(ObjectId::parse).transpose()?;
    if let Some(id) = new {
        let store = s.store(&repo);
        if !blocking(move || Ok(transport::is_complete(&store, &id)?)).await? {
            return Err(ApiError::bad(format!(
                "commit {id} or its objects are missing; send them first"
            )));
        }
    }
    // The policy in force is the one on the bookmark as it is now.
    let policy = s.policy(&repo, old.or(new)).await?;
    let rules = policy.for_bookmark(&body.name);
    if old.is_some() && rules.merge != "direct" {
        return Err(ApiError::forbidden(
            "policy",
            format!(
                "`{}` only changes through reviews (.hb/policy.toml); push another bookmark and open a review",
                body.name
            ),
        ));
    }
    check_agent_paths(&s, &repo, &who, &policy, old, new).await?;
    s.move_bookmark(&repo, &who, &body.name, body.old.clone(), body.new.clone())
        .await?;
    if let Some(commit) = new {
        // Checks for the push, and for open reviews from this bookmark.
        tokio::spawn(checks::trigger_for_push(
            Arc::clone(&s),
            Arc::clone(&repo),
            commit,
            body.name.clone(),
        ));
        let s2 = Arc::clone(&s);
        let repo2 = Arc::clone(&repo);
        let source = body.name.clone();
        tokio::spawn(async move {
            if let Err(e) = s2.retrigger_reviews(&repo2, &source, commit).await {
                tracing::warn!(error = e.message, "could not queue review checks");
            }
        });
    }
    Ok(axum::Json(json!({ "name": body.name, "target": body.new })))
}
