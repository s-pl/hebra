//! An npm registry, at `/npm/`: `npm publish`, `npm install`, dist-tags
//! and unpublishing, for scoped packages (`@owner/name`) whose scope is an
//! account here. Private unless made public.
//!
//! ```ini
//! # .npmrc
//! @acme:registry=https://hebra.example.com/npm/
//! //hebra.example.com/npm/:_authToken=hbm_…
//! ```

use std::collections::BTreeMap;

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};

use crate::api::St;
use crate::api::registry::{base64_bytes, caller, may_publish, may_read};
use crate::db::{Account, Package};
use crate::error::ApiError;
use crate::server::Server;

fn reply(status: StatusCode, body: Value) -> Response {
    (status, axum::Json(body)).into_response()
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    reply(status, json!({ "error": message.into() }))
}

fn from_api(e: &ApiError) -> Response {
    error(e.status, e.message().to_owned())
}

/// `@owner/name` → (owner, name).
fn split_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix('@')?;
    let (scope, pkg) = rest.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 214
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._~".contains(&b))
            && !s.starts_with('.')
    };
    (ok(scope) && ok(pkg)).then_some((scope, pkg))
}

fn tarball_url(s: &Server, name: &str, version: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    format!("{}/npm/{name}/-/{base}-{version}.tgz", s.public_url)
}

/// A version's manifest as clients get it: our tarball URL.
fn served(s: &Server, name: &str, version: &str, meta: &Value) -> Value {
    let mut m = meta.clone();
    m["dist"]["tarball"] = json!(tarball_url(s, name, version));
    m
}

async fn owner_of(s: &Server, scope: &str) -> Result<Account, Response> {
    s.db.account_by_handle(scope)
        .await
        .map_err(|e| from_api(&e))?
        .filter(|a| !a.disabled)
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                format!("no account `{scope}` on this server"),
            )
        })
}

pub(crate) async fn handle(
    State(s): St,
    method: Method,
    headers: HeaderMap,
    UrlPath(path): UrlPath<String>,
    body: Bytes,
) -> Response {
    match route(&s, &method, &headers, &path, body).await {
        Ok(r) | Err(r) => r,
    }
}

async fn route(
    s: &Server,
    method: &Method,
    headers: &HeaderMap,
    path: &str,
    body: Bytes,
) -> Result<Response, Response> {
    let who = caller(s, headers).await.map_err(|e| from_api(&e))?;
    let path = path.trim_matches('/');
    if path == "-/ping" {
        return Ok(reply(StatusCode::OK, json!({})));
    }
    if path == "-/whoami" {
        return match &who {
            Some(w) => Ok(reply(
                StatusCode::OK,
                json!({ "username": w.account.handle }),
            )),
            None => Err(error(
                StatusCode::UNAUTHORIZED,
                "sign in: put a mandate in .npmrc as _authToken",
            )),
        };
    }
    if let Some(rest) = path.strip_prefix("-/package/") {
        let (name, tag) = match rest.split_once("/dist-tags") {
            Some((name, tag)) => (name, tag.trim_start_matches('/')),
            None => return Err(error(StatusCode::NOT_FOUND, "unknown endpoint")),
        };
        return dist_tags(s, method, who.as_deref(), name, tag, &body).await;
    }
    // `/name/-/file.tgz`, maybe with `/-rev/…` (deleting a tarball).
    if let Some((name, file)) = path.split_once("/-/") {
        let file = file.split("/-rev/").next().unwrap_or(file);
        return if method == Method::DELETE {
            // The versions were removed by the PUT before: nothing to do.
            Ok(reply(StatusCode::OK, json!({ "ok": true })))
        } else {
            tarball(s, who.as_deref(), name, file).await
        };
    }
    if let Some((name, _rev)) = path.split_once("/-rev/") {
        let (owner, package) = writable(s, who.as_deref(), name).await?;
        let _ = owner;
        if let Some(p) = package {
            s.db.delete_package(&p.id).await.map_err(|e| from_api(&e))?;
        }
        return Ok(reply(StatusCode::OK, json!({ "ok": true })));
    }
    match *method {
        Method::GET | Method::HEAD => packument(s, who.as_deref(), path).await,
        Method::PUT => publish(s, who.as_deref(), path, &body).await,
        _ => Err(error(StatusCode::METHOD_NOT_ALLOWED, "not supported")),
    }
}

async fn readable(
    s: &Server,
    who: Option<&crate::identity::Principal>,
    name: &str,
) -> Result<Package, Response> {
    let not_found = || {
        error(
            StatusCode::NOT_FOUND,
            format!("{name} is not in this registry"),
        )
    };
    let (scope, _) = split_name(name).ok_or_else(not_found)?;
    let owner = owner_of(s, scope).await?;
    let package =
        s.db.package("npm", name)
            .await
            .map_err(|e| from_api(&e))?
            .ok_or_else(not_found)?;
    if !may_read(s, who, &owner, Some(&package))
        .await
        .map_err(|e| from_api(&e))?
    {
        // Private packages are not there for anyone else.
        return Err(not_found());
    }
    Ok(package)
}

async fn writable(
    s: &Server,
    who: Option<&crate::identity::Principal>,
    name: &str,
) -> Result<(Account, Option<Package>), Response> {
    let (scope, _) = split_name(name).ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            "packages here are scoped by an account: @owner/name",
        )
    })?;
    let owner = owner_of(s, scope).await?;
    if who.is_none() {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "sign in: put a mandate in .npmrc as _authToken",
        ));
    }
    if !may_publish(s, who, &owner)
        .await
        .map_err(|e| from_api(&e))?
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            format!("you cannot publish packages of @{scope}"),
        ));
    }
    let package = s.db.package("npm", name).await.map_err(|e| from_api(&e))?;
    Ok((owner, package))
}

async fn packument(
    s: &Server,
    who: Option<&crate::identity::Principal>,
    name: &str,
) -> Result<Response, Response> {
    let package = readable(s, who, name).await?;
    let versions = s.db.versions(&package.id).await.map_err(|e| from_api(&e))?;
    let tags = s.db.tags(&package.id).await.map_err(|e| from_api(&e))?;
    let mut all = serde_json::Map::new();
    let mut time = serde_json::Map::new();
    time.insert("created".to_owned(), json!(package.created_at));
    time.insert("modified".to_owned(), json!(package.updated_at));
    for v in &versions {
        all.insert(v.version.clone(), served(s, name, &v.version, &v.meta));
        time.insert(v.version.clone(), json!(v.created_at));
    }
    let tags: BTreeMap<String, String> = tags.into_iter().collect();
    Ok(reply(
        StatusCode::OK,
        json!({
            "_id": name,
            "name": name,
            "description": package.description,
            "dist-tags": tags,
            "versions": all,
            "time": time,
        }),
    ))
}

async fn tarball(
    s: &Server,
    who: Option<&crate::identity::Principal>,
    name: &str,
    file: &str,
) -> Result<Response, Response> {
    let package = readable(s, who, name).await?;
    let base = name.rsplit('/').next().unwrap_or(name);
    let version = file
        .strip_prefix(&format!("{base}-"))
        .and_then(|v| v.strip_suffix(".tgz"))
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "no such file"))?;
    let v =
        s.db.version(&package.id, version)
            .await
            .map_err(|e| from_api(&e))?
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    format!("{name}@{version} is not here"),
                )
            })?;
    let (size, stream) = s
        .files
        .get(&v.digest)
        .await
        .map_err(|e| from_api(&e))?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "the tarball is gone"))?;
    let mut r = Body::from_stream(stream).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    r.headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    Ok(r)
}

async fn publish(
    s: &Server,
    who: Option<&crate::identity::Principal>,
    name: &str,
    body: &Bytes,
) -> Result<Response, Response> {
    let (owner, existing) = writable(s, who, name).await?;
    let by = who.map(|w| w.account.handle.clone()).unwrap_or_default();
    let doc: Value = serde_json::from_slice(body)
        .map_err(|e| error(StatusCode::BAD_REQUEST, format!("not JSON: {e}")))?;
    if doc["name"].as_str().is_some_and(|n| n != name) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "the name in the body is not the package's",
        ));
    }
    let attachments = doc["_attachments"].as_object().filter(|a| !a.is_empty());
    let Some(attachments) = attachments else {
        // No tarball: a metadata update, which is how `npm unpublish
        // name@version` removes a version.
        let Some(package) = existing else {
            return Err(error(
                StatusCode::NOT_FOUND,
                format!("{name} is not in this registry"),
            ));
        };
        let keep: Vec<String> = doc["versions"]
            .as_object()
            .map(|v| v.keys().cloned().collect())
            .unwrap_or_default();
        for v in s.db.versions(&package.id).await.map_err(|e| from_api(&e))? {
            if !keep.contains(&v.version) {
                s.db.delete_version(&package.id, &v.version)
                    .await
                    .map_err(|e| from_api(&e))?;
            }
        }
        if let Some(tags) = doc["dist-tags"].as_object() {
            for (tag, version) in tags {
                if let Some(v) = version.as_str() {
                    s.db.set_tag(&package.id, tag, v)
                        .await
                        .map_err(|e| from_api(&e))?;
                }
            }
        }
        return Ok(reply(StatusCode::OK, json!({ "ok": true })));
    };
    let package =
        s.db.package_or_create(&owner.id, "npm", name)
            .await
            .map_err(|e| from_api(&e))?;
    let versions = doc["versions"].as_object().cloned().unwrap_or_default();
    if versions.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "nothing to publish"));
    }
    for (version, manifest) in &versions {
        if s.db
            .version(&package.id, version)
            .await
            .map_err(|e| from_api(&e))?
            .is_some()
        {
            return Err(error(
                StatusCode::FORBIDDEN,
                format!("{name}@{version} is already published; versions are never replaced"),
            ));
        }
        let base = name.rsplit('/').next().unwrap_or(name);
        let file = format!("{base}-{version}.tgz");
        let attachment = attachments
            .get(&file)
            .or_else(|| attachments.values().next())
            .ok_or_else(|| error(StatusCode::BAD_REQUEST, "no tarball for this version"))?;
        let tarball = attachment["data"]
            .as_str()
            .and_then(base64_bytes)
            .ok_or_else(|| error(StatusCode::BAD_REQUEST, "the tarball is not base64"))?;
        // What npm computed before sending must match what arrived.
        if let Some(integrity) = manifest["dist"]["integrity"].as_str()
            && let Some(expected) = integrity.strip_prefix("sha512-")
        {
            let actual = Sha512::digest(&tarball);
            if base64_bytes(expected).as_deref() != Some(actual.as_slice()) {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "the tarball does not match its integrity",
                ));
            }
        }
        let (digest, size) = s.files.put_bytes(tarball).await.map_err(|e| from_api(&e))?;
        let mut meta = manifest.clone();
        meta["dist"]["tarball"] = json!(tarball_url(s, name, version));
        s.db.add_version(&package.id, version, &digest, size, &meta, &by)
            .await
            .map_err(|e| from_api(&e))?;
    }
    if let Some(tags) = doc["dist-tags"].as_object() {
        for (tag, version) in tags {
            if let Some(v) = version.as_str() {
                s.db.set_tag(&package.id, tag, v)
                    .await
                    .map_err(|e| from_api(&e))?;
            }
        }
    }
    if let Some(d) = doc["description"].as_str() {
        s.db.set_package(
            &package.id,
            None,
            Some(&d.chars().take(500).collect::<String>()),
        )
        .await
        .map_err(|e| from_api(&e))?;
    }
    Ok(reply(StatusCode::OK, json!({ "ok": true, "id": name })))
}

async fn dist_tags(
    s: &Server,
    method: &Method,
    who: Option<&crate::identity::Principal>,
    name: &str,
    tag: &str,
    body: &Bytes,
) -> Result<Response, Response> {
    if *method == Method::GET {
        let package = readable(s, who, name).await?;
        let tags: BTreeMap<String, String> =
            s.db.tags(&package.id)
                .await
                .map_err(|e| from_api(&e))?
                .into_iter()
                .collect();
        return Ok(reply(StatusCode::OK, json!(tags)));
    }
    let (_, package) = writable(s, who, name).await?;
    let package = package.ok_or_else(|| {
        error(
            StatusCode::NOT_FOUND,
            format!("{name} is not in this registry"),
        )
    })?;
    if tag.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "which tag?"));
    }
    match *method {
        Method::PUT | Method::POST => {
            let version: String = serde_json::from_slice(body)
                .map_err(|_| error(StatusCode::BAD_REQUEST, "send the version as a JSON string"))?;
            if s.db
                .version(&package.id, &version)
                .await
                .map_err(|e| from_api(&e))?
                .is_none()
            {
                return Err(error(
                    StatusCode::NOT_FOUND,
                    format!("{name}@{version} is not published"),
                ));
            }
            s.db.set_tag(&package.id, tag, &version)
                .await
                .map_err(|e| from_api(&e))?;
        }
        Method::DELETE => {
            s.db.delete_tag(&package.id, tag)
                .await
                .map_err(|e| from_api(&e))?;
        }
        _ => return Err(error(StatusCode::METHOD_NOT_ALLOWED, "not supported")),
    }
    Ok(reply(StatusCode::OK, json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_scoped_by_an_account() {
        assert_eq!(split_name("@acme/ui"), Some(("acme", "ui")));
        assert_eq!(split_name("@acme/ui-kit.js"), Some(("acme", "ui-kit.js")));
        assert!(split_name("left-pad").is_none());
        assert!(split_name("@Acme/ui").is_none());
        assert!(split_name("@acme/../x").is_none());
    }
}
