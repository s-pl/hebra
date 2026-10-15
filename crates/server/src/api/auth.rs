//! Signing in: GitHub (web), one-time invitation links, and the device flow
//! the CLI uses (`hb login`). Web sign-ins return a session secret for the
//! web app to keep in an httpOnly cookie; the device flow returns a mandate.

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::config::Signup;
use crate::db::{Account, AccountKind};
use crate::error::{ApiError, ApiResult};
use crate::identity::{github, secret};
use crate::names::{handle_from, valid_handle};
use crate::server::Server;

/// Web sessions last a week (the web app renews by signing in again).
const SESSION_TTL_MS: i64 = 7 * 24 * 3600 * 1000;
/// Device codes are valid for 15 minutes.
const DEVICE_TTL_MS: i64 = 15 * 60 * 1000;
/// Mandates for the CLI last 90 days.
const CLI_MANDATE_SECS: i64 = 90 * 24 * 3600;

async fn open_session(s: &Server, account: &Account, headers: &HeaderMap) -> ApiResult<Value> {
    let token = secret::token(secret::SESSION_PREFIX);
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    s.db.create_session(&secret::hash(&token), &account.id, SESSION_TTL_MS, agent)
        .await?;
    Ok(json!({ "session": token, "account": account, "expires_in": SESSION_TTL_MS / 1000 }))
}

/// A free handle derived from `wanted` (`ana`, `ana-2`, …).
async fn free_handle(s: &Server, wanted: &str) -> ApiResult<String> {
    let base = handle_from(wanted);
    for n in 0..100 {
        let candidate = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        if valid_handle(&candidate) && s.db.account_by_handle(&candidate).await?.is_none() {
            return Ok(candidate);
        }
    }
    Err(ApiError::conflict(
        "could not find a free name; set one explicitly",
    ))
}

/// The first person on a server administers it.
async fn first_person_is_admin(s: &Server, account: &Account) -> ApiResult<()> {
    if s.db.count_humans().await? == 1 {
        s.db.set_site_admin(&account.id, true).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub(crate) struct GithubBody {
    code: String,
    redirect_uri: String,
}

pub(crate) async fn github(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<GithubBody>,
) -> ApiResult<axum::Json<Value>> {
    let config = s
        .github
        .as_ref()
        .ok_or_else(|| ApiError::bad("signing in with GitHub is not configured on this server"))?;
    if !body.redirect_uri.starts_with(&s.public_url) {
        return Err(ApiError::bad("redirect URI does not belong to this server"));
    }
    let user = github::exchange(&s.http, config, &body.code, &body.redirect_uri).await?;
    let account = match s.db.account_by_identity("github", &user.id).await? {
        Some(account) => account,
        None => {
            if s.signup == Signup::Invite && s.db.count_humans().await? > 0 {
                return Err(ApiError::forbidden(
                    "invite_only",
                    "this server is invite-only; ask an administrator for an invitation link",
                ));
            }
            let handle = free_handle(&s, &user.login).await?;
            let account =
                s.db.create_account(
                    &handle,
                    AccountKind::Human,
                    user.name.as_deref().unwrap_or(&user.login),
                    user.email.as_deref(),
                    None,
                )
                .await?;
            s.db.link_identity("github", &user.id, &account.id).await?;
            first_person_is_admin(&s, &account).await?;
            account
        }
    };
    if account.disabled {
        return Err(ApiError::forbidden("disabled", "this account is disabled"));
    }
    s.db.update_profile(&account.id, &account.name, user.avatar_url.as_deref())
        .await?;
    Ok(axum::Json(open_session(&s, &account, &headers).await?))
}

#[derive(Deserialize)]
pub(crate) struct InviteBody {
    code: String,
}

pub(crate) async fn invite(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<InviteBody>,
) -> ApiResult<axum::Json<Value>> {
    let account_id = s
        .db
        .redeem_invitation(&secret::hash(body.code.trim()))
        .await?
        .ok_or_else(|| ApiError::unauthorized("this sign-in link is invalid, used or expired"))?;
    let account =
        s.db.account(&account_id)
            .await?
            .filter(|a| !a.disabled)
            .ok_or_else(|| ApiError::unauthorized("this account is disabled"))?;
    Ok(axum::Json(open_session(&s, &account, &headers).await?))
}

pub(crate) async fn logout(State(s): St, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| t.starts_with(secret::SESSION_PREFIX))
    {
        s.db.delete_session(&secret::hash(token)).await?;
        s.caches.principals.remove(&secret::hash(token));
    }
    Ok(axum::Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct DeviceStart {
    #[serde(default)]
    client_name: String,
}

/// Starts `hb login`: a secret for the CLI to poll with, and a short code
/// for the person to approve in the browser.
pub(crate) async fn device_start(
    State(s): St,
    axum::Json(body): axum::Json<DeviceStart>,
) -> ApiResult<axum::Json<Value>> {
    let device_code = secret::token(secret::DEVICE_PREFIX);
    let user_code = secret::user_code();
    let client: String = body.client_name.chars().take(80).collect();
    s.db.create_device_code(
        &secret::hash(&device_code),
        &user_code,
        if client.is_empty() { "hb" } else { &client },
        DEVICE_TTL_MS,
    )
    .await?;
    Ok(axum::Json(json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": format!("{}/login/device", s.public_url),
        "verification_uri_complete": format!("{}/login/device?code={user_code}", s.public_url),
        "interval": 3,
        "expires_in": DEVICE_TTL_MS / 1000,
    })))
}

#[derive(Deserialize)]
pub(crate) struct DeviceToken {
    device_code: String,
}

/// Polled by the CLI: `pending`, `denied`, or the mandate once approved.
pub(crate) async fn device_token(
    State(s): St,
    axum::Json(body): axum::Json<DeviceToken>,
) -> ApiResult<axum::Json<Value>> {
    let Some((status, account_id)) =
        s.db.poll_device_code(&secret::hash(&body.device_code))
            .await?
    else {
        return Err(ApiError::bad("this sign-in expired; run `hb login` again"));
    };
    match (status.as_str(), account_id) {
        ("approved", Some(account_id)) => {
            let account =
                s.db.account(&account_id)
                    .await?
                    .ok_or_else(|| ApiError::unauthorized("account not found"))?;
            let expires = jiff::Timestamp::now().as_second() + CLI_MANDATE_SECS;
            let (mandate, root) = s
                .authority
                .issue(&account.id, Some(expires))
                .map_err(|e| ApiError::internal("mandate", e))?;
            s.db.record_mandate(
                &root,
                &account.id,
                "hb login",
                Some(&account.handle),
                Some(expires * 1000),
            )
            .await?;
            Ok(axum::Json(
                json!({ "status": "approved", "mandate": mandate, "account": account }),
            ))
        }
        (status, _) => Ok(axum::Json(json!({ "status": status }))),
    }
}

/// Shows a pending code to the signed-in person who is about to approve it.
pub(crate) async fn device_show(
    State(s): St,
    headers: HeaderMap,
    UrlPath(code): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    if who.is_agent() {
        return Err(ApiError::forbidden(
            "needs_human",
            "only people approve sign-ins",
        ));
    }
    let device =
        s.db.device_by_user_code(&code.trim().to_uppercase())
            .await?
            .ok_or_else(|| ApiError::not_found("this code is invalid or expired"))?;
    Ok(axum::Json(
        json!({ "client_name": device.client_name, "status": device.status, "account": who.account.handle }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct Decision {
    approve: bool,
}

pub(crate) async fn device_decide(
    State(s): St,
    headers: HeaderMap,
    UrlPath(code): UrlPath<String>,
    axum::Json(body): axum::Json<Decision>,
) -> ApiResult<axum::Json<Value>> {
    let who: Arc<_> = s.require_principal(&headers).await?;
    if who.is_agent() || who.mandate.is_some() {
        return Err(ApiError::forbidden(
            "needs_human",
            "approve sign-ins from the web, signed in as yourself",
        ));
    }
    if !s
        .db
        .decide_device_code(&code.trim().to_uppercase(), &who.account.id, body.approve)
        .await?
    {
        return Err(ApiError::bad(
            "this code is invalid, expired or already used",
        ));
    }
    Ok(axum::Json(json!({ "ok": true, "approved": body.approve })))
}
