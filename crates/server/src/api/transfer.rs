//! Moving objects: packs for pulls, object uploads for pushes.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use hebra_core::{ObjectId, transport};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{St, blocking};
use crate::error::{ApiError, ApiResult};
use crate::identity::Action;

#[derive(Deserialize)]
pub(crate) struct PackRequest {
    want: Vec<String>,
    #[serde(default)]
    have: Vec<String>,
}

/// The objects reachable from `want` that the client lacks (`have`).
pub(crate) async fn pack(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<PackRequest>,
) -> ApiResult<Response> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    if body.want.len() > 10_000 || body.have.len() > 100_000 {
        return Err(ApiError::bad("too many ids in one request"));
    }
    let store = s.store(&repo);
    blocking(move || {
        let parse = |ids: &[String]| {
            ids.iter()
                .map(|i| ObjectId::parse(i))
                .collect::<Result<Vec<_>, _>>()
        };
        let want = parse(&body.want)?;
        let have: HashSet<ObjectId> = parse(&body.have)?
            .into_iter()
            .filter(|h| store.has(h))
            .collect();
        for w in &want {
            if !store.has(w) {
                return Err(ApiError::not_found(format!("object {w} not found")));
            }
        }
        let ids = transport::closure(&store, &want, &have)?;
        let bytes = transport::encode(&transport::collect(&store, &ids)?);
        Ok(([(header::CONTENT_TYPE, "application/x-hebra-pack")], bytes).into_response())
    })
    .await
}

/// Stores objects for a push. Objects are verified (their id is their hash)
/// and only become reachable when a bookmark moves to them.
pub(crate) async fn receive_objects(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Upload).await?;
    let store = s.store(&repo);
    let (received, new) = blocking(move || {
        let objects = transport::decode(&body)?;
        let new = transport::store_all(&store, &objects)?;
        Ok((objects.len(), new))
    })
    .await?;
    s.metrics
        .objects_received
        .fetch_add(received as u64, Ordering::Relaxed);
    Ok(axum::Json(json!({ "received": received, "new": new })))
}
