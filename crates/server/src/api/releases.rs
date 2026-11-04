//! Releases (notes and files on a tag) and webhooks.

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{NewRelease, Repo};
use crate::error::{ApiError, ApiResult};
use crate::identity::{Action, Principal};
use crate::server::Server;

/// Largest release file.
const MAX_ASSET: u64 = 4 * 1024 * 1024 * 1024;
/// Events a webhook can ask for.
pub(crate) const EVENTS: [&str; 6] = ["push", "review", "task", "check", "release", "ping"];

/// People who can push, and the owner's runners (CI publishing a release).
async fn writer(
    s: &Server,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> ApiResult<(std::sync::Arc<Repo>, String)> {
    match s.access(headers, owner, name, Action::Review).await {
        Ok((repo, who)) => Ok((repo, who.map(|w| w.actor()).unwrap_or_default())),
        Err(first) => match s.access(headers, owner, name, Action::Run).await {
            Ok((repo, who)) => Ok((repo, who.map(|w| w.actor()).unwrap_or_default())),
            Err(_) => Err(first),
        },
    }
}

fn valid_asset(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
}

pub(crate) async fn list(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Read).await?;
    let can_write = s
        .authorize(who.as_deref(), &repo, Action::Review)
        .await
        .is_ok();
    let mut out = Vec::new();
    for r in s.db.releases(&repo.id).await? {
        if r.draft && !can_write {
            continue;
        }
        let assets = s.db.assets(&r.id).await?;
        let mut v = json!(r);
        v["assets"] = json!(assets);
        out.push(v);
    }
    Ok(axum::Json(json!({ "releases": out })))
}

#[derive(Deserialize)]
pub(crate) struct NewBody {
    tag: String,
    title: Option<String>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

pub(crate) async fn create(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<NewBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, by) = writer(&s, &headers, &owner, &name).await?;
    let commit = s
        .bookmark(&repo, &format!("tags/{}", body.tag))
        .await?
        .ok_or_else(|| {
            ApiError::bad(format!(
                "there is no tag `{}`; push it first (hb tag {} && hb push --tags)",
                body.tag, body.tag
            ))
        })?;
    if s.db.release(&repo.id, &body.tag).await?.is_some() {
        return Err(ApiError::conflict(format!(
            "{} already has a release",
            body.tag
        )));
    }
    // An annotated tag is a commit carrying it: the release is of its target.
    let store = s.store(&repo);
    let commit = crate::api::blocking(move || {
        let c: hebra_core::model::Commit = store.read_json(hebra_core::store::COMMIT, &commit)?;
        Ok(match (&c.tag, c.parents.as_slice()) {
            (Some(_), [target]) => *target,
            _ => commit,
        })
    })
    .await?;
    let title = body.title.unwrap_or_else(|| body.tag.clone());
    s.db.create_release(&NewRelease {
        repo_id: &repo.id,
        tag: &body.tag,
        commit: &commit.hex(),
        title: &title,
        notes: &body.notes,
        draft: body.draft,
        prerelease: body.prerelease,
        by: &by,
    })
    .await?;
    if !body.draft {
        s.notify(
            &repo,
            "release",
            json!({ "action": "published", "tag": body.tag, "title": title, "commit": commit.hex() }),
        );
    }
    Ok(axum::Json(json!(s.db.release(&repo.id, &body.tag).await?)))
}

pub(crate) async fn show(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, tag)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Read).await?;
    let release =
        s.db.release(&repo.id, &tag)
            .await?
            .ok_or_else(|| ApiError::not_found("release not found"))?;
    if release.draft
        && s.authorize(who.as_deref(), &repo, Action::Review)
            .await
            .is_err()
    {
        return Err(ApiError::not_found("release not found"));
    }
    let mut v = json!(release);
    v["assets"] = json!(s.db.assets(&release.id).await?);
    Ok(axum::Json(v))
}

#[derive(Deserialize)]
pub(crate) struct EditBody {
    title: Option<String>,
    notes: Option<String>,
    draft: Option<bool>,
    prerelease: Option<bool>,
}

pub(crate) async fn edit(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, tag)): UrlPath<(String, String, String)>,
    axum::Json(body): axum::Json<EditBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = writer(&s, &headers, &owner, &name).await?;
    let release =
        s.db.release(&repo.id, &tag)
            .await?
            .ok_or_else(|| ApiError::not_found("release not found"))?;
    s.db.update_release(
        &release.id,
        body.title.as_deref(),
        body.notes.as_deref(),
        body.draft,
        body.prerelease,
    )
    .await?;
    if release.draft && body.draft == Some(false) {
        s.notify(
            &repo,
            "release",
            json!({ "action": "published", "tag": tag, "title": release.title }),
        );
    }
    Ok(axum::Json(json!(s.db.release(&repo.id, &tag).await?)))
}

pub(crate) async fn remove(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, tag)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = writer(&s, &headers, &owner, &name).await?;
    let release =
        s.db.release(&repo.id, &tag)
            .await?
            .ok_or_else(|| ApiError::not_found("release not found"))?;
    s.db.delete_release(&release.id).await?;
    Ok(axum::Json(json!({ "removed": tag })))
}

pub(crate) async fn upload(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, tag, asset)): UrlPath<(String, String, String, String)>,
    body: Body,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = writer(&s, &headers, &owner, &name).await?;
    if !valid_asset(&asset) {
        return Err(ApiError::bad(
            "file names use letters, digits, - _ . + (up to 200)",
        ));
    }
    let release =
        s.db.release(&repo.id, &tag)
            .await?
            .ok_or_else(|| ApiError::not_found("release not found"))?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .chars()
        .take(100)
        .collect::<String>();
    let (digest, size) = s
        .files
        .put_stream(body.into_data_stream(), MAX_ASSET)
        .await?;
    s.db.add_asset(&release.id, &asset, size, &digest, &content_type)
        .await?;
    Ok(axum::Json(
        json!({ "name": asset, "size": size, "digest": format!("sha256:{digest}") }),
    ))
}

pub(crate) async fn download(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, tag, asset)): UrlPath<(String, String, String, String)>,
) -> ApiResult<Response> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let release =
        s.db.release(&repo.id, &tag)
            .await?
            .ok_or_else(|| ApiError::not_found("release not found"))?;
    let found =
        s.db.assets(&release.id)
            .await?
            .into_iter()
            .find(|a| a.name == asset)
            .ok_or_else(|| ApiError::not_found("file not found"))?;
    let (size, stream) = s
        .files
        .get(&found.digest)
        .await?
        .ok_or_else(|| ApiError::not_found("the file is gone"))?;
    s.db.count_download(&release.id, &asset).await?;
    let mut r = Body::from_stream(stream).into_response();
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&found.content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{asset}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(r)
}

pub(crate) async fn remove_asset(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, tag, asset)): UrlPath<(String, String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = writer(&s, &headers, &owner, &name).await?;
    let release =
        s.db.release(&repo.id, &tag)
            .await?
            .ok_or_else(|| ApiError::not_found("release not found"))?;
    Ok(axum::Json(
        json!({ "removed": s.db.delete_asset(&release.id, &asset).await? }),
    ))
}

// --------------------------------------------------------------- webhooks

fn hook_place(repo_id: &str, id: &str) -> String {
    format!("webhook/{repo_id}/{id}")
}

pub(crate) async fn hooks(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let mut out = Vec::new();
    for h in s.db.webhooks(&repo.id).await? {
        let last = s.db.deliveries(&h.id).await?.into_iter().next();
        let mut v = json!(h);
        v["last_delivery"] = json!(last);
        out.push(v);
    }
    Ok(axum::Json(json!({ "webhooks": out, "events": EVENTS })))
}

#[derive(Deserialize)]
pub(crate) struct HookBody {
    url: String,
    /// Signs each delivery; made up when absent and shown once.
    secret: Option<String>,
    #[serde(default)]
    events: Vec<String>,
}

pub(crate) async fn create_hook(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<HookBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    if !(body.url.starts_with("https://") || body.url.starts_with("http://"))
        || body.url.len() > 2000
    {
        return Err(ApiError::bad("the URL starts with https:// (or http://)"));
    }
    let events = if body.events.is_empty() {
        vec![
            "push".to_owned(),
            "review".to_owned(),
            "check".to_owned(),
            "release".to_owned(),
        ]
    } else {
        body.events
    };
    if let Some(bad) = events
        .iter()
        .find(|e| *e != "*" && !EVENTS.contains(&e.as_str()))
    {
        return Err(ApiError::bad(format!(
            "unknown event `{bad}` (use {} or *)",
            EVENTS.join(", ")
        )));
    }
    let secret = body
        .secret
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::identity::secret::token("whsec_"));
    // The id is part of where the secret is sealed: made first.
    let by = who.as_deref().map(Principal::actor).unwrap_or_default();
    let id =
        s.db.create_webhook(&repo.id, &body.url, "", &events, &by)
            .await?;
    let sealed = s
        .sealer
        .seal(&hook_place(&repo.id, &id), secret.as_bytes())?;
    sqlx_seal(&s, &id, &sealed).await?;
    Ok(axum::Json(
        json!({ "id": id, "url": body.url, "events": events, "secret": secret }),
    ))
}

async fn sqlx_seal(s: &Server, id: &str, sealed: &str) -> ApiResult<()> {
    s.db.seal_webhook(id, sealed).await
}

#[derive(Deserialize)]
pub(crate) struct HookEdit {
    active: Option<bool>,
    events: Option<Vec<String>>,
}

async fn hook_in(s: &Server, repo: &Repo, id: &str) -> ApiResult<crate::db::Webhook> {
    s.db.webhook(id)
        .await?
        .filter(|h| h.repo_id == repo.id)
        .ok_or_else(|| ApiError::not_found("webhook not found"))
}

pub(crate) async fn edit_hook(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
    axum::Json(body): axum::Json<HookEdit>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    hook_in(&s, &repo, &id).await?;
    s.db.set_webhook(&id, body.active, body.events.as_deref())
        .await?;
    Ok(axum::Json(json!(s.db.webhook(&id).await?)))
}

pub(crate) async fn remove_hook(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    hook_in(&s, &repo, &id).await?;
    s.db.delete_webhook(&id).await?;
    Ok(axum::Json(json!({ "removed": id })))
}

pub(crate) async fn deliveries(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    hook_in(&s, &repo, &id).await?;
    Ok(axum::Json(
        json!({ "deliveries": s.db.deliveries(&id).await? }),
    ))
}

#[derive(Deserialize, Default)]
pub(crate) struct PingQuery {}

pub(crate) async fn ping(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
    Query(_): Query<PingQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let hook = hook_in(&s, &repo, &id).await?;
    let payload = json!({ "event": "ping", "repo": repo.full_name, "data": { "hook": id }, "at": crate::db::rfc3339(crate::db::now_ms()) });
    let status = crate::webhooks::deliver(&s, &hook, "ping", &payload, 1).await;
    Ok(axum::Json(json!({ "status": status })))
}

pub(crate) fn place(repo_id: &str, id: &str) -> String {
    hook_place(repo_id, id)
}
