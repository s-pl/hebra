//! Moving objects: packs for pulls, object uploads for pushes.

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::error::{ApiError, ApiResult};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

use axum::body::Bytes;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use hebra_core::{ObjectId, transport};

use crate::api::blocking;

#[derive(Deserialize)]
pub(crate) struct PackRequest {
    want: Vec<String>,
    #[serde(default)]
    have: Vec<String>,
}

pub(crate) async fn pack(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<PackRequest>,
) -> ApiResult<Response> {
    s.require_repo(&repo, &headers).await?;
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

pub(crate) async fn receive_objects(
    State(s): St,
    UrlPath(repo): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    s.require_repo(&repo, &headers).await?;
    s.require_token(&headers).await?;
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
