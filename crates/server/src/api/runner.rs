//! Runners: registration (by people who manage the owner) and the protocol a
//! runner speaks: claim a run, renew its lease, stream the log, report the
//! result, and — for agent checks — talk to the model gateway.

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::checks::{LEASE_MS, MAX_LOG_BYTES};
use crate::config::Edition;
use crate::db::{Account, AccountKind, CheckRun, Runner};
use crate::error::{ApiError, ApiResult};
use crate::gateway;
use crate::identity::Principal;
use crate::names::valid_handle;
use crate::server::Server;

/// How long a runner's mandate lasts (re-register to renew).
const RUNNER_MANDATE_SECS: i64 = 365 * 24 * 3600;

async fn managed_owner(s: &Server, who: &Principal, handle: &str) -> ApiResult<Account> {
    let owner =
        s.db.account_by_handle(handle)
            .await?
            .filter(|a| !a.disabled)
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    if !s.manages(who, &owner, false).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you do not manage `{handle}`"),
        ));
    }
    Ok(owner)
}

pub(crate) async fn list(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed_owner(&s, &who, &handle).await?;
    Ok(axum::Json(
        json!({ "runners": s.db.runners(&owner.id).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct Register {
    name: String,
    #[serde(default)]
    labels: Vec<String>,
}

/// Registers a runner for an owner and returns its mandate (shown once).
pub(crate) async fn register(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<Register>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed_owner(&s, &who, &handle).await?;
    if !valid_handle(&body.name) {
        return Err(ApiError::bad(
            "a runner name has 1–39 lowercase letters, digits or hyphens",
        ));
    }
    if body.labels.len() > 20 || body.labels.iter().any(|l| l.is_empty() || l.len() > 50) {
        return Err(ApiError::bad("up to 20 labels of 1–50 characters"));
    }
    let account =
        s.db.create_account(
            &format!("{}-runner-{}", owner.handle, body.name),
            AccountKind::Runner,
            &body.name,
            None,
            Some(&owner.id),
        )
        .await?;
    let id =
        s.db.create_runner(&owner.id, &account.id, &body.name, &body.labels)
            .await?;
    let expires = jiff::Timestamp::now().as_second() + RUNNER_MANDATE_SECS;
    let (mandate, root) = s
        .authority
        .issue(&account.id, Some(expires))
        .map_err(|e| ApiError::internal("mandate", e))?;
    s.db.record_mandate(
        &root,
        &account.id,
        "runner",
        Some(&who.account.handle),
        Some(expires * 1000),
    )
    .await?;
    Ok(axum::Json(
        json!({ "id": id, "name": body.name, "labels": body.labels, "mandate": mandate }),
    ))
}

pub(crate) async fn remove(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed_owner(&s, &who, &handle).await?;
    let runner =
        s.db.runners(&owner.id)
            .await?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or_else(|| ApiError::not_found(format!("runner `{name}` not found")))?;
    s.db.disable_account(&runner.account_id).await?;
    s.db.delete_runner(&runner.id).await?;
    s.caches.principals.clear();
    Ok(axum::Json(json!({ "removed": name })))
}

// -------------------------------------------------------------- protocol

/// The runner behind a request (runner mandates only).
async fn runner(s: &Server, headers: &HeaderMap) -> ApiResult<(Arc<Principal>, Runner)> {
    let who = s.require_principal(headers).await?;
    if who.account.kind != AccountKind::Runner {
        return Err(ApiError::forbidden(
            "forbidden",
            "this is for runners (`hb runner start`)",
        ));
    }
    let runner =
        s.db.runner_by_account(&who.account.id)
            .await?
            .ok_or_else(|| ApiError::forbidden("forbidden", "this runner was removed"))?;
    Ok((who, runner))
}

async fn held_run(s: &Server, runner: &Runner, id: &str) -> ApiResult<CheckRun> {
    s.db.run(id)
        .await?
        .filter(|r| r.runner_id.as_deref() == Some(runner.id.as_str()) && r.state == "running")
        .ok_or_else(|| {
            ApiError::conflict("this run is no longer yours (cancelled, or its lease expired)")
        })
}

#[derive(Deserialize)]
pub(crate) struct ClaimBody {
    #[serde(default)]
    version: String,
}

/// Leases the next run for this runner, if any. Runners poll this.
pub(crate) async fn claim(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<ClaimBody>,
) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = runner(&s, &headers).await?;
    s.db.runner_seen(&runner.id, body.version.get(..40).unwrap_or(&body.version))
        .await?;
    let Some(run) = s.db.claim_run(&runner, LEASE_MS).await? else {
        return Ok(axum::Json(json!({ "run": null })));
    };
    let repo =
        s.db.repo_by_id(&run.repo_id)
            .await?
            .ok_or_else(|| ApiError::internal("checks", "run without repository"))?;
    s.notify(
        &repo,
        "check",
        json!({ "id": run.id, "name": run.name, "state": "running" }),
    );
    Ok(axum::Json(json!({
        "run": run,
        "repo": repo.full_name,
        "lease_ms": LEASE_MS,
    })))
}

pub(crate) async fn heartbeat(
    State(s): St,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = runner(&s, &headers).await?;
    let alive = s.db.renew_run(&id, &runner.id, LEASE_MS).await?;
    Ok(axum::Json(json!({ "continue": alive })))
}

#[derive(Deserialize)]
pub(crate) struct LogBody {
    text: String,
}

pub(crate) async fn log(
    State(s): St,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    axum::Json(body): axum::Json<LogBody>,
) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = runner(&s, &headers).await?;
    let run = held_run(&s, &runner, &id).await?;
    if body.text.len() > 256 * 1024 {
        return Err(ApiError::bad("send logs in chunks of up to 256 KiB"));
    }
    let kept = s.db.append_log(&run.id, &body.text, MAX_LOG_BYTES).await?;
    Ok(axum::Json(json!({ "kept": kept })))
}

#[derive(Deserialize)]
pub(crate) struct Complete {
    /// `success`, `failure` or `error`.
    state: String,
    exit_code: Option<i64>,
    #[serde(default)]
    summary: String,
}

pub(crate) async fn complete(
    State(s): St,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    axum::Json(body): axum::Json<Complete>,
) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = runner(&s, &headers).await?;
    if !["success", "failure", "error"].contains(&body.state.as_str()) {
        return Err(ApiError::bad("state is success, failure or error"));
    }
    let run = held_run(&s, &runner, &id).await?;
    let summary: String = body.summary.chars().take(16 * 1024).collect();
    if !s
        .db
        .finish_run(
            &run.id,
            Some(&runner.id),
            &body.state,
            body.exit_code,
            &summary,
            0,
        )
        .await?
    {
        return Err(ApiError::conflict("this run is no longer yours"));
    }
    if let Some(repo) = s.db.repo_by_id(&run.repo_id).await? {
        s.notify(
            &repo,
            "check",
            json!({ "id": run.id, "name": run.name, "state": body.state, "commit": run.commit }),
        );
    }
    Ok(axum::Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct AgentTurn {
    messages: Value,
}

/// One model turn for an agent check: the runner sends the conversation,
/// the gateway answers with the model's reply and the credits left.
pub(crate) async fn agent(
    State(s): St,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    axum::Json(body): axum::Json<AgentTurn>,
) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = runner(&s, &headers).await?;
    let run = held_run(&s, &runner, &id).await?;
    if run.kind != "agent" {
        return Err(ApiError::bad("this run is not an agent check"));
    }
    let gw = match (&s.gateway, s.edition) {
        (Some(gw), Edition::Cloud) => gw,
        _ => {
            return Err(ApiError::bad(
                "agent verification is not available on this server",
            ));
        }
    };
    if body.messages.to_string().len() > gateway::MAX_REQUEST_BYTES {
        return Err(ApiError::bad(
            "conversation too large; summarise tool output",
        ));
    }
    gateway::validate_messages(&body.messages)?;
    let repo =
        s.db.repo_by_id(&run.repo_id)
            .await?
            .ok_or_else(|| ApiError::internal("checks", "run without repository"))?;
    let owner =
        s.db.account(&repo.owner_id)
            .await?
            .ok_or_else(|| ApiError::internal("checks", "repository without owner"))?;
    let run_cap = run.spec["max_credits"].as_i64().unwrap_or(500);
    if run.credits >= run_cap {
        return Err(ApiError::payment(format!(
            "this run reached its limit of {run_cap} credits"
        )));
    }
    if let Some(left) = s.credits_left(&owner).await?
        && left <= 0
    {
        return Err(ApiError::payment(format!(
            "`{}` has no agent credits left this period; raise the spending limit in billing settings",
            owner.handle
        )));
    }
    let reply = gateway::complete(&s.http, gw, &body.messages).await?;
    let markup = s.plans.as_ref().map_or(30, |p| p.model_markup_percent);
    let credits = gateway::credits_for(&reply["usage"], gateway::Prices::default(), markup);
    s.db.add_run_credits(&run.id, credits).await?;
    s.charge_credits(&owner, credits, &format!("check:{}", run.id))
        .await?;
    Ok(axum::Json(json!({
        "content": reply["content"],
        "stop_reason": reply["stop_reason"],
        "credits": credits,
        "run_credits": run.credits + credits,
        "run_limit": run_cap,
    })))
}
