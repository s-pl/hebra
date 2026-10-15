//! Moving bookmarks: compare-and-swap, checked against the token's patterns.

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};
use hebra_core::{ObjectId, transport};

use crate::api::blocking;
use crate::auth::valid_bookmark;

#[derive(Deserialize)]
pub(crate) struct UpdateBookmark {
    name: String,
    old: Option<String>,
    new: Option<String>,
}

pub(crate) async fn update_bookmark(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpdateBookmark>,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let token = s.require_token(&headers).await?;
    if !valid_bookmark(&body.name) {
        return Err(ApiError::bad(format!(
            "invalid bookmark name `{}`",
            body.name
        )));
    }
    s.check_move(&repo, &token, &body.name).await?;
    if let Some(new) = &body.new {
        let id = ObjectId::parse(new)?;
        let store = s.store(&repo);
        if !blocking(move || Ok(transport::is_complete(&store, &id)?)).await? {
            return Err(ApiError::bad(format!(
                "commit {new} or its objects are missing; send them first"
            )));
        }
    }
    s.move_bookmark(&repo, &token, &body.name, body.old, body.new.clone())
        .await?;
    Ok(axum::Json(json!({ "name": body.name, "target": body.new })))
}
