//! Probes, metrics and "who am I".

use axum::extract::State;
use axum::http::HeaderMap;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::ApiResult;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

pub(crate) async fn health() -> impl IntoResponse {
    axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Ready to take traffic: the database answers and the event bus is up.
pub(crate) async fn ready(State(s): St) -> Response {
    let db = s.meta.ping().await.is_ok();
    let bus = s.bus.healthy();
    let status = if db && bus {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, axum::Json(json!({ "database": db, "events": bus }))).into_response()
}

pub(crate) async fn me(State(s): St, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let token = s.require_token(&headers).await?;
    Ok(axum::Json(json!({
        "name": token.name,
        "kind": token.kind,
        "on_behalf_of": token.on_behalf_of,
        "allow": token.allow,
        "admin": token.admin,
        "display": token.display(),
    })))
}

pub(crate) async fn metrics(State(s): St) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render(&s),
    )
        .into_response()
}
