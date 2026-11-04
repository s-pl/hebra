//! Environments: where checks deploy to, their rules, secrets and history.

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::EnvironmentRules;
use crate::error::{ApiError, ApiResult};
use crate::identity::Action;

/// Secrets of an environment live under this owner in the repository scope.
pub(crate) fn env_owner(repo_id: &str, env: &str) -> String {
    format!("{repo_id}@{env}")
}

fn valid_env(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 50
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub(crate) async fn list(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let mut out = Vec::new();
    for env in s.db.environments(&repo.id).await? {
        let last = s.db.deployments(&repo.id, Some(&env.name), 1).await?;
        let mut v = json!(env);
        v["last_deployment"] = json!(last.first());
        out.push(v);
    }
    // Environments named in deployments but never configured.
    for d in s.db.deployments(&repo.id, None, 200).await? {
        if !out.iter().any(|e| e["name"] == d.environment) {
            out.push(json!({ "name": d.environment, "required_approvals": 0, "branches": [], "runner_labels": [], "configured": false, "last_deployment": d }));
        }
    }
    Ok(axum::Json(json!({ "environments": out })))
}

#[derive(Deserialize)]
pub(crate) struct Rules {
    #[serde(default)]
    required_approvals: i64,
    #[serde(default)]
    branches: Vec<String>,
    #[serde(default)]
    runner_labels: Vec<String>,
    url: Option<String>,
}

pub(crate) async fn put(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, env)): UrlPath<(String, String, String)>,
    axum::Json(body): axum::Json<Rules>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    if !valid_env(&env) {
        return Err(ApiError::bad(
            "environment names are lowercase letters, digits, - and _ (up to 50)",
        ));
    }
    if !(0..=10).contains(&body.required_approvals) {
        return Err(ApiError::bad("required_approvals is 0 to 10"));
    }
    if body.branches.len() > 20 || body.runner_labels.len() > 20 {
        return Err(ApiError::bad("up to 20 branches and 20 runner labels"));
    }
    s.db.put_environment(
        &repo.id,
        &env,
        &EnvironmentRules {
            required_approvals: body.required_approvals,
            branches: &body.branches,
            runner_labels: &body.runner_labels,
            url: body.url.as_deref(),
        },
    )
    .await?;
    Ok(axum::Json(json!(s.db.environment(&repo.id, &env).await?)))
}

pub(crate) async fn remove(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, env)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    Ok(axum::Json(
        json!({ "removed": s.db.delete_environment(&repo.id, &env).await? }),
    ))
}

pub(crate) async fn deployments(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, env)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    Ok(axum::Json(json!({
        "environment": s.db.environment(&repo.id, &env).await?,
        "deployments": s.db.deployments(&repo.id, Some(&env), 100).await?,
    })))
}

pub(crate) async fn secrets(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, env)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    Ok(axum::Json(
        json!({ "secrets": s.db.secrets("repo", &env_owner(&repo.id, &env)).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct SecretBody {
    value: String,
}

pub(crate) async fn put_secret(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, env, secret)): UrlPath<(String, String, String, String)>,
    axum::Json(body): axum::Json<SecretBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    if !valid_env(&env) {
        return Err(ApiError::bad("invalid environment name"));
    }
    super::pipelines::store_secret(
        &s,
        "repo",
        &env_owner(&repo.id, &env),
        &secret,
        &body.value,
        who.as_deref(),
    )
    .await
}

pub(crate) async fn delete_secret(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, env, secret)): UrlPath<(String, String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let removed =
        s.db.delete_secret("repo", &env_owner(&repo.id, &env), &secret)
            .await?;
    Ok(axum::Json(json!({ "removed": removed })))
}

/// A maintainer approves a deployment waiting for approval; it starts once
/// it has as many as its environment requires.
pub(crate) async fn approve(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s
        .access(&headers, &owner, &name, Action::ApproveChecks)
        .await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    if who.is_agent() {
        return Err(ApiError::forbidden(
            "forbidden",
            "deployments are approved by people",
        ));
    }
    let run =
        s.db.run(&id)
            .await?
            .filter(|r| r.repo_id == repo.id)
            .ok_or_else(|| ApiError::not_found("check run not found"))?;
    let env =
        s.db.run_environment(&run.id)
            .await?
            .ok_or_else(|| ApiError::bad("this run does not deploy anywhere"))?;
    if run.state != "waiting" {
        return Err(ApiError::conflict(format!(
            "this deployment is {}, not waiting",
            run.state
        )));
    }
    let needed =
        s.db.environment(&repo.id, &env)
            .await?
            .map_or(0, |e| e.required_approvals);
    let approvals =
        s.db.approve_deployment(&run.id, &who.account.handle)
            .await?;
    let released = i64::try_from(approvals.len()).unwrap_or(i64::MAX) >= needed
        && s.db.release_run(&run.id).await?;
    if released {
        s.notify(&repo, "check", json!({ "id": run.id, "state": "queued" }));
    }
    Ok(axum::Json(json!({
        "environment": env,
        "approved_by": approvals,
        "required": needed,
        "released": released,
    })))
}
