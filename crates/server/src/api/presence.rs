//! Who is working where right now.

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::now_ms;
use crate::error::{ApiError, ApiResult};
use crate::identity::Action;
use crate::server::PRESENCE_TTL_MS;

pub(crate) async fn get(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let presence = s.db.presence(&repo.id, now_ms() - PRESENCE_TTL_MS).await?;
    Ok(axum::Json(json!({ "presence": presence })))
}

#[derive(Deserialize)]
pub(crate) struct PresenceBody {
    client: String,
    #[serde(flatten)]
    payload: Value,
}

/// Largest presence report accepted (workspaces and their changes).
const MAX_PAYLOAD: usize = 256 * 1024;

pub(crate) async fn post(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<PresenceBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Read).await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    if body.client.is_empty() || body.client.len() > 200 {
        return Err(ApiError::bad("invalid client id"));
    }
    let mut payload = body.payload;
    payload["token"] = json!(who.actor());
    if payload.to_string().len() > MAX_PAYLOAD {
        return Err(ApiError::bad("presence report too large"));
    }
    s.db.put_presence(&repo.id, &body.client, &payload).await?;
    s.notify(
        &repo,
        "presence",
        json!({ "client": body.client, "actor": who.actor() }),
    );
    Ok(axum::Json(json!({ "ok": true })))
}
