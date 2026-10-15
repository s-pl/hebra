//! Repositories: listing, creation, information and their event history.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};

pub(crate) async fn list_repos(State(s): St) -> ApiResult<axum::Json<Value>> {
    Ok(axum::Json(json!({ "repos": s.meta.public_repos().await? })))
}

#[derive(Deserialize)]
pub(crate) struct CreateRepo {
    name: String,
    #[serde(default)]
    protect: Option<Vec<String>>,
}

pub(crate) async fn create_repo(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CreateRepo>,
) -> ApiResult<axum::Json<Value>> {
    let token = s.require_token(&headers).await?;
    if !token.admin {
        return Err(ApiError::forbidden(
            "forbidden",
            "only admin tokens can create repositories",
        ));
    }
    let protect = body.protect.unwrap_or_else(|| vec!["main".to_owned()]);
    s.create_repo(&body.name, &protect)
        .await
        .map_err(|e| ApiError::bad(e.to_string()))?;
    Ok(axum::Json(
        json!({ "name": body.name, "protected": protect }),
    ))
}

pub(crate) async fn repo_info(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    Ok(axum::Json(json!({
        "name": repo,
        "bookmarks": *s.bookmarks(&repo).await?,
        "protected": *s.protected(&repo).await?,
    })))
}

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    limit: Option<usize>,
}

pub(crate) async fn events(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let limit = i64::try_from(q.limit.unwrap_or(30).min(200)).unwrap_or(30);
    Ok(axum::Json(
        json!({ "events": s.meta.events(&repo, limit).await? }),
    ))
}
