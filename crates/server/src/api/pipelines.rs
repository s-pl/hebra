//! Pipelines: secrets, artifacts, machines that register themselves as
//! runners, the queue as an orchestrator sees it, and Heddle's insights.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{Account, Repo, RunStat};
use crate::error::{ApiError, ApiResult};
use crate::identity::{Action, Principal, secret};
use crate::names::valid_handle;
use crate::server::Server;

/// Largest artifact accepted.
const MAX_ARTIFACT: u64 = 2 * 1024 * 1024 * 1024;
/// Largest secret value.
const MAX_SECRET: usize = 64 * 1024;
/// Runner tokens start with this.
pub(crate) const RUNNER_TOKEN_PREFIX: &str = "hbr_";

fn valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && !name.starts_with("HEBRA_")
}

async fn managed(s: &Server, who: &Principal, handle: &str) -> ApiResult<Account> {
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

// ---------------------------------------------------------------- secrets

#[derive(Deserialize)]
pub(crate) struct SecretBody {
    value: String,
}

fn place(scope: &str, owner: &str, name: &str) -> String {
    format!("{scope}/{owner}/{name}")
}

pub(crate) async fn repo_secrets(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    Ok(axum::Json(json!({
        "secrets": s.db.secrets("repo", &repo.id).await?,
        "inherited": s.db.secrets("account", &repo.owner_id).await?,
    })))
}

pub(crate) async fn put_repo_secret(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, secret_name)): UrlPath<(String, String, String)>,
    axum::Json(body): axum::Json<SecretBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    store_secret(
        &s,
        "repo",
        &repo.id,
        &secret_name,
        &body.value,
        who.as_deref(),
    )
    .await
}

pub(crate) async fn delete_repo_secret(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, secret_name)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let removed = s.db.delete_secret("repo", &repo.id, &secret_name).await?;
    Ok(axum::Json(json!({ "removed": removed })))
}

pub(crate) async fn account_secrets(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    Ok(axum::Json(
        json!({ "secrets": s.db.secrets("account", &owner.id).await? }),
    ))
}

pub(crate) async fn put_account_secret(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, secret_name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<SecretBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    store_secret(
        &s,
        "account",
        &owner.id,
        &secret_name,
        &body.value,
        Some(&who),
    )
    .await
}

pub(crate) async fn delete_account_secret(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, secret_name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let removed =
        s.db.delete_secret("account", &owner.id, &secret_name)
            .await?;
    Ok(axum::Json(json!({ "removed": removed })))
}

pub(crate) async fn store_secret(
    s: &Server,
    scope: &str,
    owner_id: &str,
    name: &str,
    value: &str,
    who: Option<&Principal>,
) -> ApiResult<axum::Json<Value>> {
    if !valid_secret_name(name) {
        return Err(ApiError::bad(
            "secret names are UPPER_CASE letters, digits and _, not starting with HEBRA_",
        ));
    }
    if value.len() > MAX_SECRET {
        return Err(ApiError::bad("a secret holds up to 64 KiB"));
    }
    let sealed = s
        .sealer
        .seal(&place(scope, owner_id, name), value.as_bytes())?;
    let by = who.map(|w| w.account.handle.clone()).unwrap_or_default();
    s.db.put_secret(scope, owner_id, name, &sealed, &by).await?;
    Ok(axum::Json(json!({ "name": name, "scope": scope })))
}

/// Opened secrets for a run (only for runners, only names the check asks
/// for, never for contributions waiting for approval): the environment's,
/// else the repository's, else its owner's.
pub(crate) async fn run_secrets(
    s: &Server,
    repo: &Repo,
    environment: Option<&str>,
    names: &[String],
) -> ApiResult<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut rest = Vec::new();
    for name in names {
        let env_owner = environment.map(|e| super::environments::env_owner(&repo.id, e));
        match &env_owner {
            Some(owner) => match s.db.sealed_secret("repo", owner, name).await? {
                Some(sealed) => {
                    let value = s.sealer.open(&place("repo", owner, name), &sealed)?;
                    out.insert(name.clone(), String::from_utf8_lossy(&value).into_owned());
                }
                None => rest.push(name.clone()),
            },
            None => rest.push(name.clone()),
        }
    }
    for (name, scope, owner, sealed) in s.db.sealed_secrets(&repo.id, &repo.owner_id, &rest).await?
    {
        let value = s.sealer.open(&place(&scope, &owner, &name), &sealed)?;
        out.insert(name, String::from_utf8_lossy(&value).into_owned());
    }
    Ok(out)
}

// -------------------------------------------------------------- artifacts

#[derive(Deserialize)]
pub(crate) struct ArtifactQuery {
    name: String,
}

fn valid_artifact_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
}

/// A runner uploads one artifact (a file, usually a `.tar.gz`) of a run it holds.
pub(crate) async fn upload_artifact(
    State(s): St,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<ArtifactQuery>,
    body: Body,
) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = super::runner::runner(&s, &headers).await?;
    let run = super::runner::held_run(&s, &runner, &id).await?;
    if !valid_artifact_name(&q.name) {
        return Err(ApiError::bad(
            "artifact names use letters, digits, - _ . + (up to 200)",
        ));
    }
    let (digest, size) = s
        .files
        .put_stream(body.into_data_stream(), MAX_ARTIFACT)
        .await?;
    let artifact =
        s.db.add_artifact(&run.id, &run.repo_id, &q.name, size, &digest)
            .await?;
    Ok(axum::Json(json!(artifact)))
}

pub(crate) async fn artifacts(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let run =
        s.db.run(&id)
            .await?
            .filter(|r| r.repo_id == repo.id)
            .ok_or_else(|| ApiError::not_found("check run not found"))?;
    Ok(axum::Json(
        json!({ "artifacts": s.db.artifacts(&run.id).await? }),
    ))
}

pub(crate) async fn download_artifact(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id, artifact)): UrlPath<(String, String, String, String)>,
) -> ApiResult<Response> {
    // Runners of the owner fetch artifacts of the runs theirs depend on.
    let (repo, _) = match s.access(&headers, &owner, &name, Action::Read).await {
        Ok(found) => found,
        Err(e) => return Err(e),
    };
    let run =
        s.db.run(&id)
            .await?
            .filter(|r| r.repo_id == repo.id)
            .ok_or_else(|| ApiError::not_found("check run not found"))?;
    let found =
        s.db.artifact(&run.id, &artifact)
            .await?
            .ok_or_else(|| ApiError::not_found("artifact not found"))?;
    let (size, stream) = s
        .files
        .get(&found.digest)
        .await?
        .ok_or_else(|| ApiError::not_found("artifact content is gone"))?;
    let mut response = Body::from_stream(stream).into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", found.name)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(response)
}

// ---------------------------------------------------- machines as runners

#[derive(Deserialize)]
pub(crate) struct TokenBody {
    #[serde(default)]
    labels: Vec<String>,
    /// The runner takes one run and goes (machines an orchestrator starts).
    #[serde(default)]
    ephemeral: bool,
    /// How many machines may register with it.
    uses: Option<i64>,
    /// Seconds it stays valid.
    ttl_secs: Option<i64>,
}

/// A one-time token a machine registers with (`hb runner register --token`),
/// so a VPS or an orchestrator needs no person's credential.
pub(crate) async fn create_runner_token(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<TokenBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    check_labels(&body.labels)?;
    let uses = body.uses.unwrap_or(1).clamp(1, 1000);
    let ttl = body.ttl_secs.unwrap_or(3600).clamp(60, 30 * 24 * 3600);
    let token = secret::token(RUNNER_TOKEN_PREFIX);
    let expires = crate::db::now_ms() + ttl * 1000;
    s.db.create_runner_token(&crate::db::NewRunnerToken {
        hash: &secret::hash(&token),
        owner_id: &owner.id,
        labels: &body.labels,
        ephemeral: body.ephemeral,
        uses,
        expires_at: expires,
        by: &who.account.handle,
    })
    .await?;
    Ok(axum::Json(json!({
        "token": token,
        "owner": owner.handle,
        "labels": body.labels,
        "ephemeral": body.ephemeral,
        "uses": uses,
        "expires_at": crate::db::rfc3339(expires),
    })))
}

fn check_labels(labels: &[String]) -> ApiResult<()> {
    if labels.len() > 20 || labels.iter().any(|l| l.is_empty() || l.len() > 50) {
        return Err(ApiError::bad("up to 20 labels of 1–50 characters"));
    }
    Ok(())
}

#[derive(Deserialize)]
pub(crate) struct TokenRegister {
    token: String,
    name: String,
    #[serde(default)]
    labels: Vec<String>,
    /// The provider's id of the machine (orchestrated runners).
    machine: Option<String>,
}

/// A machine registers itself with a runner token: no session needed.
pub(crate) async fn register_with_token(
    State(s): St,
    axum::Json(body): axum::Json<TokenRegister>,
) -> ApiResult<axum::Json<Value>> {
    if !valid_handle(&body.name) {
        return Err(ApiError::bad(
            "a runner name has 1–39 lowercase letters, digits or hyphens",
        ));
    }
    check_labels(&body.labels)?;
    let grant =
        s.db.take_runner_token(&secret::hash(&body.token))
            .await?
            .ok_or_else(|| {
                ApiError::unauthorized("this runner token is unknown, used up or expired")
            })?;
    let owner =
        s.db.account(&grant.owner_id)
            .await?
            .ok_or_else(|| ApiError::not_found("the token's owner is gone"))?;
    let mut labels = grant.labels.clone();
    for l in body.labels {
        if !labels.contains(&l) {
            labels.push(l);
        }
    }
    let registered = super::runner::create(&s, &owner, &body.name, &labels, "token").await?;
    s.db.mark_runner(&registered.0, grant.ephemeral, body.machine.as_deref())
        .await?;
    Ok(axum::Json(json!({
        "id": registered.0,
        "name": body.name,
        "owner": owner.handle,
        "labels": labels,
        "ephemeral": grant.ephemeral,
        "mandate": registered.1,
    })))
}

/// A runner removes itself (an ephemeral machine on its way out).
pub(crate) async fn deregister(State(s): St, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (_, runner) = super::runner::runner(&s, &headers).await?;
    s.db.disable_account(&runner.account_id).await?;
    s.db.delete_runner(&runner.id).await?;
    s.caches.principals.clear();
    Ok(axum::Json(json!({ "removed": runner.name })))
}

/// The queue and the runners of an owner, for an orchestrator deciding
/// which machines to start or stop.
pub(crate) async fn queue(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let runners = s.db.runners(&owner.id).await?;
    let busy = s.db.busy_runners(&owner.id).await?;
    let runners: Vec<Value> = runners
        .into_iter()
        .map(|r| {
            let running = busy
                .iter()
                .find(|(id, _)| *id == r.id)
                .map(|(_, run)| run.clone());
            let mut v = json!(r);
            v["busy"] = json!(running.is_some());
            v["run"] = json!(running);
            v
        })
        .collect();
    Ok(axum::Json(json!({
        "owner": owner.handle,
        "queued": s.db.queued_runs(&owner.id).await?,
        "runners": runners,
    })))
}

// ------------------------------------------------------------------ Heddle

fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted.get(i).copied()
}

/// What Heddle knows about each check: how long it takes and waits, how
/// often it fails or flakes, and what reuse saved.
#[must_use]
pub fn insights(history: &[RunStat]) -> Value {
    let mut by: BTreeMap<String, Vec<&RunStat>> = BTreeMap::new();
    for r in history {
        let base = r.name.split(" (").next().unwrap_or(&r.name).to_owned();
        by.entry(base).or_default().push(r);
    }
    let mut checks = Vec::new();
    let mut saved_ms = 0i64;
    for (name, runs) in &by {
        let mut durations: Vec<i64> = runs
            .iter()
            .filter(|r| !r.reused && r.state != "skipped")
            .filter_map(|r| Some(r.finished_ms? - r.started_ms?))
            .collect();
        durations.sort_unstable();
        let mut waits: Vec<i64> = runs
            .iter()
            .filter(|r| !r.reused)
            .filter_map(|r| Some(r.started_ms? - r.queued_ms))
            .collect();
        waits.sort_unstable();
        let executed = runs
            .iter()
            .filter(|r| !r.reused && r.state != "skipped")
            .count();
        let failed = runs
            .iter()
            .filter(|r| matches!(r.state.as_str(), "failure" | "error"))
            .count();
        let flaky = runs
            .iter()
            .filter(|r| r.state == "success" && r.attempt > 1)
            .count();
        let reused = runs.iter().filter(|r| r.reused).count();
        let p50 = percentile(&durations, 0.5);
        saved_ms += p50.unwrap_or(0) * i64::try_from(reused).unwrap_or(0);
        #[allow(clippy::cast_precision_loss)]
        let rate = |n: usize| {
            if executed == 0 {
                0.0
            } else {
                n as f64 / executed as f64
            }
        };
        checks.push(json!({
            "name": name,
            "runs": runs.len(),
            "executed": executed,
            "reused": reused,
            "duration_ms": { "p50": p50, "p90": percentile(&durations, 0.9) },
            "wait_ms": { "p50": percentile(&waits, 0.5), "p90": percentile(&waits, 0.9) },
            "failure_rate": rate(failed),
            "flaky_rate": rate(flaky),
        }));
    }
    json!({ "checks": checks, "saved_ms": saved_ms, "runs": history.len() })
}

pub(crate) async fn repo_insights(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let history = s.db.run_history(&repo.id, 2000).await?;
    Ok(axum::Json(insights(&history)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(name: &str, state: &str, start: i64, end: i64, attempt: i64, reused: bool) -> RunStat {
        RunStat {
            name: name.to_owned(),
            state: state.to_owned(),
            labels: Vec::new(),
            queued_ms: 0,
            started_ms: Some(start),
            finished_ms: Some(end),
            attempt,
            reused,
        }
    }

    #[test]
    fn insights_count_durations_failures_and_reuse() {
        let h = vec![
            stat("test (linux)", "success", 10, 110, 1, false),
            stat("test (windows)", "failure", 10, 310, 1, false),
            stat("test (linux)", "success", 0, 0, 1, true),
            stat("lint", "success", 5, 25, 2, false),
        ];
        let v = insights(&h);
        let test = &v["checks"][1];
        assert_eq!(test["name"], "test");
        assert_eq!(test["executed"], 2);
        assert_eq!(test["reused"], 1);
        assert_eq!(test["failure_rate"], 0.5);
        assert_eq!(v["checks"][0]["flaky_rate"], 1.0);
        assert!(v["saved_ms"].as_i64().unwrap() >= 100);
        assert!(valid_secret_name("NPM_TOKEN"));
        assert!(!valid_secret_name("HEBRA_X") && !valid_secret_name("lower"));
    }
}
