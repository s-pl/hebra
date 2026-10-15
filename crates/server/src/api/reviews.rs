//! Reviews: open, list, show (with mergeability and diff), merge, close.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};
use std::sync::atomic::Ordering;

use axum::http::StatusCode;

use crate::api::blocking;
use crate::api::history::commit_json;
use crate::merging;
use crate::meta;
use crate::server::{Server, now};

pub(crate) async fn review_or_404(s: &Server, repo: &str, id: i64) -> ApiResult<meta::Review> {
    s.meta
        .review(repo, id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("review #{id} not found")))
}

#[derive(Deserialize)]
pub(crate) struct ReviewsQuery {
    state: Option<String>,
}

pub(crate) async fn list_reviews(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Query<ReviewsQuery>,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let state = q.state.unwrap_or_else(|| "open".to_owned());
    Ok(axum::Json(
        json!({ "reviews": s.meta.reviews(&repo, &state).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct OpenReview {
    source: String,
    target: Option<String>,
    title: String,
    #[serde(default)]
    body: String,
}

pub(crate) async fn open_review(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<OpenReview>,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let token = s.require_token(&headers).await?;
    let target = body.target.unwrap_or_else(|| "main".to_owned());
    let title = body.title.trim();
    if title.is_empty() || title.len() > 300 || body.body.len() > 64 * 1024 {
        return Err(ApiError::bad(
            "a review needs a title (up to 300 characters) and a body up to 64 KiB",
        ));
    }
    if s.bookmark(&repo, &body.source).await?.is_none() {
        return Err(ApiError::bad(format!(
            "bookmark `{}` does not exist on the server; push it first",
            body.source
        )));
    }
    if s.bookmark(&repo, &target).await?.is_none() {
        return Err(ApiError::bad(format!(
            "target bookmark `{target}` does not exist"
        )));
    }
    if let Some(id) = s.meta.open_review_for(&repo, &body.source, &target).await? {
        return Ok(axum::Json(json!(review_or_404(&s, &repo, id).await?)));
    }
    let at = now();
    let author = token.display();
    let id = s
        .meta
        .insert_review(&meta::NewReview {
            repo: &repo,
            source: &body.source,
            target: &target,
            title,
            body: &body.body,
            author: &author,
            at: &at,
        })
        .await?;
    s.meta
        .add_event(
            &repo,
            &at,
            &author,
            "review",
            &json!({ "name": body.source, "old": null, "new": null, "review": id }),
        )
        .await?;
    s.notify(&repo, "review", json!({ "id": id, "state": "open" }));
    Ok(axum::Json(json!(review_or_404(&s, &repo, id).await?)))
}

pub(crate) async fn get_review(
    State(s): St,
    UrlPath((repo, id)): UrlPath<(String, i64)>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let review = review_or_404(&s, &repo, id).await?;
    let source = s.bookmark(&repo, &review.source).await?;
    let target = s.bookmark(&repo, &review.target).await?;
    let mut out = json!(review);
    if let (Some(source), Some(target)) = (source, target) {
        let store = s.store(&repo);
        let (plan, files, commits) = blocking(move || {
            let plan = merging::plan(&store, target, source)?;
            let files = merging::changes(&store, plan.base, source)?;
            let commits: Vec<Value> = merging::commits(&store, target, source, 50)?
                .iter()
                .map(|(id, c)| commit_json(id, c))
                .collect();
            Ok((plan, files, commits))
        })
        .await?;
        out["source_commit"] = json!(source.hex());
        out["target_commit"] = json!(target.hex());
        out["mergeability"] = json!(plan.mergeability);
        out["files"] = json!(files);
        out["commits"] = json!(commits);
    }
    Ok(axum::Json(out))
}

pub(crate) async fn merge_review(
    State(s): St,
    UrlPath((repo, id)): UrlPath<(String, i64)>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let token = s.require_token(&headers).await?;
    let review = review_or_404(&s, &repo, id).await?;
    if review.state != "open" {
        return Err(ApiError::bad(format!("review #{id} is {}", review.state)));
    }
    let (source_name, target_name) = (review.source.clone(), review.target.clone());
    s.check_move(&repo, &token, &target_name).await?;
    let source = s
        .bookmark_fresh(&repo, &source_name)
        .await?
        .ok_or_else(|| ApiError::bad(format!("`{source_name}` no longer exists")))?;
    let target = s
        .bookmark_fresh(&repo, &target_name)
        .await?
        .ok_or_else(|| ApiError::bad(format!("`{target_name}` no longer exists")))?;
    let store = s.store(&repo);
    let approver = token.clone();
    let (src, tgt) = (source_name.clone(), target_name.clone());
    let title = review.title.clone();
    // The merge commit (if one is needed) is written first; it only becomes
    // visible when the bookmark moves.
    let merged = blocking(move || {
        let plan = merging::plan(&store, target, source)?;
        match (&plan.mergeability, &plan.tree) {
            (merging::Mergeability::UpToDate, _) => Ok(target),
            (merging::Mergeability::FastForward, _) => Ok(source),
            (merging::Mergeability::Clean, Some(tree)) => Ok(merging::write_merge(
                &store, target, source, tree, &title, &src, &tgt, &approver,
            )?),
            (merging::Mergeability::Conflicts { paths }, _) => Err(ApiError::new(
                StatusCode::CONFLICT,
                "conflicts",
                format!(
                    "merging would conflict in {}; run `hb merge {src}@origin`, resolve, and push",
                    paths.join(", ")
                ),
            )),
            (merging::Mergeability::Clean, None) => Err(ApiError::bad("nothing to merge")),
        }
    })
    .await?;
    if merged != target {
        s.move_bookmark(
            &repo,
            &token,
            &target_name,
            Some(target.hex()),
            Some(merged.hex()),
        )
        .await?;
    }
    if !s
        .meta
        .finish_review(
            &repo,
            id,
            "merged",
            &token.display(),
            Some(&merged.hex()),
            &now(),
        )
        .await?
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "stale",
            format!("review #{id} was closed meanwhile"),
        ));
    }
    s.metrics.merges.fetch_add(1, Ordering::Relaxed);
    s.notify(&repo, "review", json!({ "id": id, "state": "merged" }));
    Ok(axum::Json(json!(review_or_404(&s, &repo, id).await?)))
}

pub(crate) async fn close_review(
    State(s): St,
    UrlPath((repo, id)): UrlPath<(String, i64)>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let token = s.require_token(&headers).await?;
    let review = review_or_404(&s, &repo, id).await?;
    if review.state != "open"
        || !s
            .meta
            .finish_review(&repo, id, "closed", &token.display(), None, &now())
            .await?
    {
        return Err(ApiError::bad(format!(
            "review #{id} is already {}",
            if review.state == "open" {
                "closed"
            } else {
                &review.state
            }
        )));
    }
    s.notify(&repo, "review", json!({ "id": id, "state": "closed" }));
    Ok(axum::Json(json!(review_or_404(&s, &repo, id).await?)))
}
