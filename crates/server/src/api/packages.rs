//! Packages as the hub shows them: an owner's packages, their versions and
//! tags, visibility, and removing versions.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::api::registry::{may_publish, may_read};
use crate::error::{ApiError, ApiResult};

#[derive(Deserialize)]
pub(crate) struct Which {
    kind: String,
    name: String,
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    kind: Option<String>,
}

pub(crate) async fn list(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.authenticate(&headers).await?;
    let owner =
        s.db.account_by_handle(&handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    let mine = may_publish(&s, who.as_deref(), &owner).await?;
    let mut out = Vec::new();
    for p in s.db.packages(&owner.id).await? {
        if !(mine || p.public) || q.kind.as_ref().is_some_and(|k| *k != p.kind) {
            continue;
        }
        let versions = s.db.versions(&p.id).await?;
        let tags = s.db.tags(&p.id).await?;
        let latest = tags
            .iter()
            .find(|(t, _)| t == "latest")
            .map(|(_, v)| v.clone())
            .or_else(|| versions.last().map(|v| v.version.clone()));
        let mut v = json!(p);
        v["versions"] = json!(versions.len());
        v["tags"] = json!(tags.len());
        v["latest"] = json!(latest);
        v["bytes"] = json!(s.db.package_bytes(&p.id).await?);
        out.push(v);
    }
    Ok(axum::Json(json!({ "packages": out, "can_publish": mine })))
}

pub(crate) async fn show(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    Query(q): Query<Which>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.authenticate(&headers).await?;
    let owner =
        s.db.account_by_handle(&handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    let p =
        s.db.package(&q.kind, &q.name)
            .await?
            .filter(|p| p.owner_id == owner.id)
            .ok_or_else(|| ApiError::not_found("package not found"))?;
    if !may_read(&s, who.as_deref(), &owner, Some(&p)).await? {
        return Err(ApiError::not_found("package not found"));
    }
    Ok(axum::Json(json!({
        "package": p,
        "versions": s.db.versions(&p.id).await?,
        "tags": s.db.tags(&p.id).await?.into_iter().map(|(tag, version)| json!({ "tag": tag, "version": version })).collect::<Vec<_>>(),
        "bytes": s.db.package_bytes(&p.id).await?,
        "can_publish": may_publish(&s, who.as_deref(), &owner).await?,
        "registry": s.public_url,
    })))
}

#[derive(Deserialize)]
pub(crate) struct Update {
    kind: String,
    name: String,
    public: Option<bool>,
    description: Option<String>,
}

pub(crate) async fn update(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<Update>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner =
        s.db.account_by_handle(&handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    if !s.manages(&who, &owner, false).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            "only administrators change packages",
        ));
    }
    let p =
        s.db.package(&body.kind, &body.name)
            .await?
            .filter(|p| p.owner_id == owner.id)
            .ok_or_else(|| ApiError::not_found("package not found"))?;
    s.db.set_package(&p.id, body.public, body.description.as_deref())
        .await?;
    Ok(axum::Json(json!(
        s.db.package(&body.kind, &body.name).await?
    )))
}

#[derive(Deserialize)]
pub(crate) struct Removal {
    kind: String,
    name: String,
    /// One version; the whole package when absent.
    version: Option<String>,
}

pub(crate) async fn remove(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    Query(q): Query<Removal>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner =
        s.db.account_by_handle(&handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    if !s.manages(&who, &owner, false).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            "only administrators delete packages",
        ));
    }
    let p =
        s.db.package(&q.kind, &q.name)
            .await?
            .filter(|p| p.owner_id == owner.id)
            .ok_or_else(|| ApiError::not_found("package not found"))?;
    match q.version {
        Some(v) => {
            if !s.db.delete_version(&p.id, &v).await? {
                return Err(ApiError::not_found("version not found"));
            }
        }
        None => s.db.delete_package(&p.id).await?,
    }
    Ok(axum::Json(json!({ "removed": true })))
}
