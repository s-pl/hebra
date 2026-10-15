//! Probes, capabilities and metrics.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::api::St;
use crate::config::{Edition, Signup};

pub(crate) async fn health() -> impl IntoResponse {
    axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Ready to take traffic: the database answers and the event bus is up.
pub(crate) async fn ready(State(s): St) -> Response {
    let db = s.db.ping().await.is_ok();
    let bus = s.bus.healthy();
    let status = if db && bus {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, axum::Json(json!({ "database": db, "events": bus }))).into_response()
}

/// What this server offers, so clients show only what works.
pub(crate) async fn capabilities(State(s): St) -> axum::Json<serde_json::Value> {
    let cloud = s.edition == Edition::Cloud;
    axum::Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "edition": s.edition,
        "public_url": s.public_url,
        "signup": match s.signup { Signup::Open => "open", Signup::Invite => "invite" },
        "sign_in": { "github": s.github.as_ref().map(|g| json!({ "client_id": g.client_id })) },
        "features": {
            "organizations": true,
            "mandates": true,
            "tasks": true,
            "reviews": true,
            "checks": true,
            "self_hosted_runners": true,
            "agent_verification": cloud && s.gateway.is_some(),
            "merge_queue": cloud,
            "cloud_runners": false,
            "billing": cloud && s.plans.is_some(),
            "transparency_witness": cloud,
        },
    }))
}

pub(crate) async fn metrics(State(s): St) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render(&s),
    )
        .into_response()
}
