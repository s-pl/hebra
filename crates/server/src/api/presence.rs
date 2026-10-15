//! Who is working where right now.

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};
use crate::server::{PRESENCE_TTL_SECS, unix_now};

pub(crate) async fn get_presence(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let presence = s
        .meta
        .presence(&repo, unix_now() - PRESENCE_TTL_SECS)
        .await?;
    Ok(axum::Json(json!({ "presence": presence })))
}

#[derive(Deserialize)]
pub(crate) struct PresenceBody {
    client: String,
    #[serde(flatten)]
    payload: Value,
}

pub(crate) async fn post_presence(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<PresenceBody>,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    let token = s.require_token(&headers).await?;
    if body.client.is_empty() || body.client.len() > 200 {
        return Err(ApiError::bad("invalid client id"));
    }
    let mut payload = body.payload;
    payload["token"] = json!(token.display());
    s.meta
        .put_presence(&repo, &body.client, &payload, unix_now())
        .await?;
    s.notify(
        &repo,
        "presence",
        json!({ "client": body.client, "actor": token.display() }),
    );
    Ok(axum::Json(json!({ "ok": true })))
}
