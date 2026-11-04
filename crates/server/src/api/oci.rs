//! A container registry (the OCI distribution API, what `docker push`,
//! `docker pull`, Podman and containerd speak), at `/v2/`. Images are
//! named `owner/name`, where `owner` is an account here; private unless
//! made public. Sign in with `docker login <server>`, any user name, a
//! mandate as the password.
//!
//! Layers and configs are stored once, by digest, in the large-file store;
//! an image can only read blobs uploaded or mounted into it.

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::api::registry::{caller, may_publish, may_read};
use crate::db::{Account, Package};
use crate::error::ApiError;
use crate::files::hex_of;
use crate::identity::Principal;
use crate::server::Server;

/// Largest manifest accepted.
const MAX_MANIFEST: usize = 4 * 1024 * 1024;
/// Largest layer accepted.
const MAX_BLOB: u64 = 50 * 1024 * 1024 * 1024;

const MANIFEST_TYPES: [&str; 4] = [
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
];

fn oci_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let mut r = (
        status,
        axum::Json(json!({ "errors": [{ "code": code, "message": message.into() }] })),
    )
        .into_response();
    if status == StatusCode::UNAUTHORIZED {
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Hebra\""),
        );
    }
    r
}

fn from_api(e: &ApiError) -> Response {
    let code = match e.status {
        StatusCode::UNAUTHORIZED => "UNAUTHORIZED",
        StatusCode::FORBIDDEN => "DENIED",
        StatusCode::NOT_FOUND => "NAME_UNKNOWN",
        StatusCode::PAYLOAD_TOO_LARGE => "SIZE_INVALID",
        _ => "UNKNOWN",
    };
    oci_error(e.status, code, e.message().to_owned())
}

fn with_headers(mut r: Response, headers: &[(header::HeaderName, String)]) -> Response {
    for (k, v) in headers {
        if let Ok(v) = HeaderValue::from_str(v) {
            r.headers_mut().insert(k.clone(), v);
        }
    }
    r.headers_mut().insert(
        "docker-distribution-api-version",
        HeaderValue::from_static("registry/2.0"),
    );
    r
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.split('/').count() >= 2
        && name.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
                && !part.starts_with(['.', '_', '-'])
        })
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Params {
    digest: Option<String>,
    mount: Option<String>,
    from: Option<String>,
    n: Option<usize>,
}

/// `GET /v2/`: whether the caller is signed in (Docker asks this first).
pub(crate) async fn root(State(s): St, headers: HeaderMap) -> Response {
    match caller(&s, &headers).await {
        Ok(Some(_)) => with_headers(axum::Json(json!({})).into_response(), &[]),
        Ok(None) => oci_error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "sign in with docker login",
        ),
        Err(e) => from_api(&e),
    }
}

pub(crate) async fn handle(
    State(s): St,
    method: Method,
    headers: HeaderMap,
    UrlPath(path): UrlPath<String>,
    Query(params): Query<Params>,
    body: Body,
) -> Response {
    match route(
        &s,
        &method,
        &headers,
        path.trim_end_matches('/'),
        &params,
        body,
    )
    .await
    {
        Ok(r) | Err(r) => r,
    }
}

/// Where in the API a path is.
enum Target<'a> {
    Catalog,
    Tags(&'a str),
    Manifest(&'a str, &'a str),
    Uploads(&'a str, Option<&'a str>),
    Blob(&'a str, &'a str),
}

fn target(path: &str) -> Option<Target<'_>> {
    if path == "_catalog" {
        return Some(Target::Catalog);
    }
    if let Some(name) = path.strip_suffix("/tags/list") {
        return Some(Target::Tags(name));
    }
    if let Some((name, reference)) = path.rsplit_once("/manifests/") {
        return Some(Target::Manifest(name, reference));
    }
    if let Some((name, rest)) = path.split_once("/blobs/uploads") {
        let id = rest.trim_matches('/');
        return Some(Target::Uploads(name, (!id.is_empty()).then_some(id)));
    }
    if let Some((name, digest)) = path.rsplit_once("/blobs/") {
        return Some(Target::Blob(name, digest));
    }
    None
}

async fn owner_of(s: &Server, name: &str) -> Result<Account, Response> {
    if !valid_name(name) {
        return Err(oci_error(
            StatusCode::BAD_REQUEST,
            "NAME_INVALID",
            "images are named owner/name, in lowercase",
        ));
    }
    let handle = name.split('/').next().unwrap_or_default();
    s.db.account_by_handle(handle)
        .await
        .map_err(|e| from_api(&e))?
        .filter(|a| !a.disabled)
        .ok_or_else(|| {
            oci_error(
                StatusCode::NOT_FOUND,
                "NAME_UNKNOWN",
                format!("no account `{handle}` here"),
            )
        })
}

/// The image, for reading (404 for private images of others).
async fn readable(s: &Server, who: Option<&Principal>, name: &str) -> Result<Package, Response> {
    let owner = owner_of(s, name).await?;
    let package =
        s.db.package("container", name)
            .await
            .map_err(|e| from_api(&e))?;
    let allowed = may_read(s, who, &owner, package.as_ref())
        .await
        .map_err(|e| from_api(&e))?;
    match package {
        Some(p) if allowed => Ok(p),
        _ if who.is_none() => Err(oci_error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "sign in with docker login",
        )),
        _ => Err(oci_error(
            StatusCode::NOT_FOUND,
            "NAME_UNKNOWN",
            format!("{name} is not here"),
        )),
    }
}

/// The image, for pushing (created on the first push).
async fn writable(s: &Server, who: Option<&Principal>, name: &str) -> Result<Package, Response> {
    let owner = owner_of(s, name).await?;
    if who.is_none() {
        return Err(oci_error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "sign in with docker login",
        ));
    }
    if !may_publish(s, who, &owner)
        .await
        .map_err(|e| from_api(&e))?
    {
        return Err(oci_error(
            StatusCode::FORBIDDEN,
            "DENIED",
            format!("you cannot push images of {}", owner.handle),
        ));
    }
    s.db.package_or_create(&owner.id, "container", name)
        .await
        .map_err(|e| from_api(&e))
}

async fn route(
    s: &Server,
    method: &Method,
    headers: &HeaderMap,
    path: &str,
    params: &Params,
    body: Body,
) -> Result<Response, Response> {
    let who = caller(s, headers).await.map_err(|e| from_api(&e))?;
    let who = who.as_deref();
    let target = target(path)
        .ok_or_else(|| oci_error(StatusCode::NOT_FOUND, "NAME_UNKNOWN", "unknown endpoint"))?;
    match target {
        Target::Catalog => catalog(s, who, params).await,
        Target::Tags(name) => {
            let p = readable(s, who, name).await?;
            let mut tags: Vec<String> =
                s.db.tags(&p.id)
                    .await
                    .map_err(|e| from_api(&e))?
                    .into_iter()
                    .map(|(t, _)| t)
                    .collect();
            if let Some(n) = params.n {
                tags.truncate(n);
            }
            Ok(with_headers(
                axum::Json(json!({ "name": name, "tags": tags })).into_response(),
                &[],
            ))
        }
        Target::Manifest(name, reference) => match *method {
            Method::GET | Method::HEAD => {
                get_manifest(s, who, name, reference, *method == Method::HEAD).await
            }
            Method::PUT => put_manifest(s, who, name, reference, headers, body).await,
            Method::DELETE => {
                let p = writable(s, who, name).await?;
                let digest = if reference.starts_with("sha256:") {
                    reference.to_owned()
                } else {
                    s.db.tag(&p.id, reference)
                        .await
                        .map_err(|e| from_api(&e))?
                        .ok_or_else(|| {
                            oci_error(StatusCode::NOT_FOUND, "MANIFEST_UNKNOWN", "no such tag")
                        })?
                };
                if !s
                    .db
                    .delete_version(&p.id, &digest)
                    .await
                    .map_err(|e| from_api(&e))?
                {
                    return Err(oci_error(
                        StatusCode::NOT_FOUND,
                        "MANIFEST_UNKNOWN",
                        "no such manifest",
                    ));
                }
                Ok(with_headers(StatusCode::ACCEPTED.into_response(), &[]))
            }
            _ => Err(oci_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                "not supported",
            )),
        },
        Target::Blob(name, digest) => match *method {
            Method::GET | Method::HEAD => {
                get_blob(s, who, name, digest, *method == Method::HEAD).await
            }
            Method::DELETE => Ok(with_headers(StatusCode::ACCEPTED.into_response(), &[])),
            _ => Err(oci_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                "not supported",
            )),
        },
        Target::Uploads(name, id) => upload(s, who, method, name, id, params, body).await,
    }
}

async fn catalog(
    s: &Server,
    who: Option<&Principal>,
    params: &Params,
) -> Result<Response, Response> {
    let mut owners = Vec::new();
    if let Some(w) = who {
        owners.push(w.account.id.clone());
        if let Some(o) = &w.owner {
            owners.push(o.id.clone());
        }
        for m in
            s.db.orgs_of(&w.account.id)
                .await
                .map_err(|e| from_api(&e))?
        {
            owners.push(m.account.id);
        }
    }
    let mut names =
        s.db.container_names(&owners)
            .await
            .map_err(|e| from_api(&e))?;
    if let Some(n) = params.n {
        names.truncate(n);
    }
    Ok(with_headers(
        axum::Json(json!({ "repositories": names })).into_response(),
        &[],
    ))
}

async fn get_blob(
    s: &Server,
    who: Option<&Principal>,
    name: &str,
    digest: &str,
    head: bool,
) -> Result<Response, Response> {
    let p = readable(s, who, name).await?;
    let hex = hex_of(digest).ok_or_else(|| {
        oci_error(
            StatusCode::BAD_REQUEST,
            "DIGEST_INVALID",
            "sha256 digests only",
        )
    })?;
    if !s.db.has_blob(&p.id, hex).await.map_err(|e| from_api(&e))? {
        return Err(oci_error(
            StatusCode::NOT_FOUND,
            "BLOB_UNKNOWN",
            "no such blob in this image",
        ));
    }
    let digest_header = (
        header::HeaderName::from_static("docker-content-digest"),
        format!("sha256:{hex}"),
    );
    if head {
        let size = s
            .files
            .size(hex)
            .await
            .map_err(|e| from_api(&e))?
            .ok_or_else(|| oci_error(StatusCode::NOT_FOUND, "BLOB_UNKNOWN", "gone"))?;
        return Ok(with_headers(
            StatusCode::OK.into_response(),
            &[
                (header::CONTENT_LENGTH, size.to_string()),
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                digest_header,
            ],
        ));
    }
    let (size, stream) = s
        .files
        .get(hex)
        .await
        .map_err(|e| from_api(&e))?
        .ok_or_else(|| oci_error(StatusCode::NOT_FOUND, "BLOB_UNKNOWN", "gone"))?;
    Ok(with_headers(
        Body::from_stream(stream).into_response(),
        &[
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            digest_header,
        ],
    ))
}

async fn resolve(
    s: &Server,
    p: &Package,
    reference: &str,
) -> Result<crate::db::PackageVersion, Response> {
    let digest = if reference.starts_with("sha256:") {
        reference.to_owned()
    } else {
        s.db.tag(&p.id, reference)
            .await
            .map_err(|e| from_api(&e))?
            .ok_or_else(|| {
                oci_error(
                    StatusCode::NOT_FOUND,
                    "MANIFEST_UNKNOWN",
                    format!("no tag `{reference}`"),
                )
            })?
    };
    s.db.version(&p.id, &digest)
        .await
        .map_err(|e| from_api(&e))?
        .ok_or_else(|| {
            oci_error(
                StatusCode::NOT_FOUND,
                "MANIFEST_UNKNOWN",
                "no such manifest",
            )
        })
}

async fn get_manifest(
    s: &Server,
    who: Option<&Principal>,
    name: &str,
    reference: &str,
    head: bool,
) -> Result<Response, Response> {
    let p = readable(s, who, name).await?;
    let v = resolve(s, &p, reference).await?;
    let media = v.meta["media_type"]
        .as_str()
        .unwrap_or(MANIFEST_TYPES[0])
        .to_owned();
    let headers = [
        (header::CONTENT_TYPE, media),
        (header::CONTENT_LENGTH, v.size.to_string()),
        (
            header::HeaderName::from_static("docker-content-digest"),
            v.version.clone(),
        ),
        (header::ETAG, format!("\"{}\"", v.version)),
    ];
    if head {
        return Ok(with_headers(StatusCode::OK.into_response(), &headers));
    }
    let bytes = s
        .files
        .read(&v.digest, MAX_MANIFEST as u64)
        .await
        .map_err(|e| from_api(&e))?
        .ok_or_else(|| oci_error(StatusCode::NOT_FOUND, "MANIFEST_UNKNOWN", "gone"))?;
    Ok(with_headers(Body::from(bytes).into_response(), &headers))
}

async fn put_manifest(
    s: &Server,
    who: Option<&Principal>,
    name: &str,
    reference: &str,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, Response> {
    let p = writable(s, who, name).await?;
    let mut bytes = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| oci_error(StatusCode::BAD_REQUEST, "MANIFEST_INVALID", e.to_string()))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_MANIFEST {
            return Err(oci_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "SIZE_INVALID",
                "manifests are up to 4 MiB",
            ));
        }
    }
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| {
        oci_error(
            StatusCode::BAD_REQUEST,
            "MANIFEST_INVALID",
            format!("not JSON: {e}"),
        )
    })?;
    let media = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_owned())
        .or_else(|| doc["mediaType"].as_str().map(str::to_owned))
        .unwrap_or_else(|| MANIFEST_TYPES[0].to_owned());
    if !MANIFEST_TYPES.contains(&media.as_str()) {
        return Err(oci_error(
            StatusCode::BAD_REQUEST,
            "MANIFEST_INVALID",
            format!("unsupported manifest type {media}"),
        ));
    }
    // Everything it points to must be here already.
    let index = media.contains("index") || media.contains("list");
    let mut wanted: Vec<String> = Vec::new();
    if index {
        for m in doc["manifests"].as_array().into_iter().flatten() {
            wanted.extend(m["digest"].as_str().map(str::to_owned));
        }
    } else {
        wanted.extend(doc["config"]["digest"].as_str().map(str::to_owned));
        for l in doc["layers"].as_array().into_iter().flatten() {
            wanted.extend(l["digest"].as_str().map(str::to_owned));
        }
    }
    for d in &wanted {
        let hex = hex_of(d).ok_or_else(|| {
            oci_error(
                StatusCode::BAD_REQUEST,
                "DIGEST_INVALID",
                format!("{d} is not a sha256 digest"),
            )
        })?;
        let present = if index {
            s.db.version(&p.id, d)
                .await
                .map_err(|e| from_api(&e))?
                .is_some()
        } else {
            s.db.has_blob(&p.id, hex).await.map_err(|e| from_api(&e))?
        };
        if !present {
            return Err(oci_error(
                StatusCode::BAD_REQUEST,
                if index {
                    "MANIFEST_UNKNOWN"
                } else {
                    "MANIFEST_BLOB_UNKNOWN"
                },
                format!("{d} was not pushed"),
            ));
        }
    }
    let size = bytes.len() as u64;
    let (hex, _) = s.files.put_bytes(bytes).await.map_err(|e| from_api(&e))?;
    let digest = format!("sha256:{hex}");
    if reference.starts_with("sha256:") && reference != digest {
        return Err(oci_error(
            StatusCode::BAD_REQUEST,
            "DIGEST_INVALID",
            "the manifest does not match its digest",
        ));
    }
    let by = who.map(|w| w.account.handle.clone()).unwrap_or_default();
    let meta = json!({
        "media_type": media,
        "layers": doc["layers"].as_array().map_or(0, Vec::len),
        "platforms": doc["manifests"].as_array().map(|m| m.iter().filter_map(|x| {
            let os = x["platform"]["os"].as_str()?;
            let arch = x["platform"]["architecture"].as_str()?;
            Some(format!("{os}/{arch}"))
        }).collect::<Vec<_>>()),
        "bytes": doc["layers"].as_array().map(|l| l.iter().filter_map(|x| x["size"].as_u64()).sum::<u64>()),
    });
    s.db.add_version(&p.id, &digest, &hex, size, &meta, &by)
        .await
        .map_err(|e| from_api(&e))?;
    if !reference.starts_with("sha256:") {
        s.db.set_tag(&p.id, reference, &digest)
            .await
            .map_err(|e| from_api(&e))?;
    }
    Ok(with_headers(
        StatusCode::CREATED.into_response(),
        &[
            (header::LOCATION, format!("/v2/{name}/manifests/{digest}")),
            (
                header::HeaderName::from_static("docker-content-digest"),
                digest,
            ),
        ],
    ))
}

#[allow(clippy::too_many_lines)]
async fn upload(
    s: &Server,
    who: Option<&Principal>,
    method: &Method,
    name: &str,
    id: Option<&str>,
    params: &Params,
    body: Body,
) -> Result<Response, Response> {
    let p = writable(s, who, name).await?;
    let location = |id: &str| format!("/v2/{name}/blobs/uploads/{id}");
    let done = |hex: &str| {
        with_headers(
            StatusCode::CREATED.into_response(),
            &[
                (header::LOCATION, format!("/v2/{name}/blobs/sha256:{hex}")),
                (
                    header::HeaderName::from_static("docker-content-digest"),
                    format!("sha256:{hex}"),
                ),
                (header::CONTENT_LENGTH, "0".to_owned()),
            ],
        )
    };
    match (method, id) {
        (&Method::POST, None) => {
            // Mount a blob from another image the caller can read.
            if let (Some(mount), Some(from)) = (&params.mount, &params.from)
                && let Some(hex) = hex_of(mount)
                && let Ok(source) = readable(s, who, from).await
                && s.db
                    .has_blob(&source.id, hex)
                    .await
                    .map_err(|e| from_api(&e))?
                && let Some(size) = s.files.size(hex).await.map_err(|e| from_api(&e))?
            {
                s.db.add_blob(&p.id, hex, size)
                    .await
                    .map_err(|e| from_api(&e))?;
                return Ok(done(hex));
            }
            // The whole blob in one request.
            if let Some(digest) = &params.digest {
                let want = hex_of(digest)
                    .ok_or_else(|| {
                        oci_error(
                            StatusCode::BAD_REQUEST,
                            "DIGEST_INVALID",
                            "sha256 digests only",
                        )
                    })?
                    .to_owned();
                let (hex, size) = s
                    .files
                    .put_stream(body.into_data_stream(), MAX_BLOB)
                    .await
                    .map_err(|e| from_api(&e))?;
                if hex != want {
                    return Err(oci_error(
                        StatusCode::BAD_REQUEST,
                        "DIGEST_INVALID",
                        "the blob does not match its digest",
                    ));
                }
                s.db.add_blob(&p.id, &hex, size)
                    .await
                    .map_err(|e| from_api(&e))?;
                return Ok(done(&hex));
            }
            let id = s.db.start_upload(&p.id).await.map_err(|e| from_api(&e))?;
            let path = s.files.upload_path(&id).ok_or_else(|| {
                oci_error(StatusCode::INTERNAL_SERVER_ERROR, "UNKNOWN", "upload path")
            })?;
            tokio::fs::File::create(&path).await.map_err(|e| {
                oci_error(StatusCode::INTERNAL_SERVER_ERROR, "UNKNOWN", e.to_string())
            })?;
            Ok(with_headers(
                StatusCode::ACCEPTED.into_response(),
                &[
                    (header::LOCATION, location(&id)),
                    (header::RANGE, "0-0".to_owned()),
                    (
                        header::HeaderName::from_static("docker-upload-uuid"),
                        id.clone(),
                    ),
                    (header::CONTENT_LENGTH, "0".to_owned()),
                ],
            ))
        }
        (_, Some(id)) => {
            let (package_id, size) =
                s.db.upload(id)
                    .await
                    .map_err(|e| from_api(&e))?
                    .filter(|(pkg, _)| *pkg == p.id)
                    .ok_or_else(|| {
                        oci_error(
                            StatusCode::NOT_FOUND,
                            "BLOB_UPLOAD_UNKNOWN",
                            "no such upload",
                        )
                    })?;
            let _ = package_id;
            let path = s.files.upload_path(id).ok_or_else(|| {
                oci_error(
                    StatusCode::NOT_FOUND,
                    "BLOB_UPLOAD_UNKNOWN",
                    "no such upload",
                )
            })?;
            let size = u64::try_from(size).unwrap_or(0);
            let range = |n: u64| format!("0-{}", n.saturating_sub(1));
            match *method {
                Method::GET => Ok(with_headers(
                    StatusCode::NO_CONTENT.into_response(),
                    &[
                        (header::LOCATION, location(id)),
                        (header::RANGE, range(size)),
                        (
                            header::HeaderName::from_static("docker-upload-uuid"),
                            id.to_owned(),
                        ),
                    ],
                )),
                Method::DELETE => {
                    s.db.end_upload(id).await.map_err(|e| from_api(&e))?;
                    let _ = tokio::fs::remove_file(&path).await;
                    Ok(with_headers(StatusCode::NO_CONTENT.into_response(), &[]))
                }
                Method::PATCH | Method::PUT => {
                    let mut file = tokio::fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .await
                        .map_err(|_| {
                            oci_error(
                                StatusCode::NOT_FOUND,
                                "BLOB_UPLOAD_UNKNOWN",
                                "the upload expired",
                            )
                        })?;
                    let total =
                        crate::files::append(&mut file, body.into_data_stream(), MAX_BLOB, size)
                            .await
                            .map_err(|e| from_api(&e))?;
                    tokio::io::AsyncWriteExt::flush(&mut file)
                        .await
                        .map_err(|e| {
                            oci_error(StatusCode::INTERNAL_SERVER_ERROR, "UNKNOWN", e.to_string())
                        })?;
                    drop(file);
                    s.db.set_upload_size(id, total)
                        .await
                        .map_err(|e| from_api(&e))?;
                    if *method == Method::PATCH {
                        return Ok(with_headers(
                            StatusCode::ACCEPTED.into_response(),
                            &[
                                (header::LOCATION, location(id)),
                                (header::RANGE, range(total)),
                                (
                                    header::HeaderName::from_static("docker-upload-uuid"),
                                    id.to_owned(),
                                ),
                                (header::CONTENT_LENGTH, "0".to_owned()),
                            ],
                        ));
                    }
                    let want = params
                        .digest
                        .as_deref()
                        .and_then(hex_of)
                        .ok_or_else(|| {
                            oci_error(
                                StatusCode::BAD_REQUEST,
                                "DIGEST_INVALID",
                                "finish an upload with ?digest=sha256:…",
                            )
                        })?
                        .to_owned();
                    let (hex, size) = s.files.put_file(&path).await.map_err(|e| from_api(&e))?;
                    s.db.end_upload(id).await.map_err(|e| from_api(&e))?;
                    if hex != want {
                        return Err(oci_error(
                            StatusCode::BAD_REQUEST,
                            "DIGEST_INVALID",
                            "the blob does not match its digest",
                        ));
                    }
                    s.db.add_blob(&p.id, &hex, size)
                        .await
                        .map_err(|e| from_api(&e))?;
                    Ok(done(&hex))
                }
                _ => Err(oci_error(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "UNSUPPORTED",
                    "not supported",
                )),
            }
        }
        _ => Err(oci_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "UNSUPPORTED",
            "not supported",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_map_to_the_api() {
        assert!(matches!(
            target("acme/app/manifests/latest"),
            Some(Target::Manifest("acme/app", "latest"))
        ));
        assert!(matches!(
            target("acme/tools/x/blobs/sha256:ab"),
            Some(Target::Blob("acme/tools/x", "sha256:ab"))
        ));
        assert!(matches!(
            target("acme/app/blobs/uploads"),
            Some(Target::Uploads("acme/app", None))
        ));
        assert!(matches!(
            target("acme/app/blobs/uploads/upl-1"),
            Some(Target::Uploads("acme/app", Some("upl-1")))
        ));
        assert!(matches!(
            target("acme/app/tags/list"),
            Some(Target::Tags("acme/app"))
        ));
        assert!(valid_name("acme/app") && valid_name("acme/tools/build"));
        assert!(!valid_name("app") && !valid_name("Acme/app") && !valid_name("acme/../x"));
    }
}
