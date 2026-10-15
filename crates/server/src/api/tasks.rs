//! Tasks: create, list (`ready` ones first for agents), claim with a lease,
//! release, close, reopen.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{NewTask, now_ms};
use crate::error::{ApiError, ApiResult};
use crate::identity::Action;

/// Shortest and longest claim.
const MIN_LEASE_SECS: i64 = 60;
const MAX_LEASE_SECS: i64 = 24 * 3600;

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    /// `open` (default), `ready`, `claimed`, `done`, `cancelled`, `all`.
    state: Option<String>,
}

pub(crate) async fn list(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    Query(q): Query<ListQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let state = q.state.unwrap_or_else(|| "open".to_owned());
    if !["open", "ready", "claimed", "done", "cancelled", "all"].contains(&state.as_str()) {
        return Err(ApiError::bad(
            "state is open, ready, claimed, done, cancelled or all",
        ));
    }
    Ok(axum::Json(
        json!({ "tasks": s.db.tasks(&repo.id, &state).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct TaskBody {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    acceptance: Vec<String>,
    #[serde(default)]
    labels: Vec<String>,
    parent: Option<i64>,
    #[serde(default)]
    depends_on: Vec<i64>,
}

fn validate(t: &TaskBody) -> ApiResult<()> {
    if t.title.trim().is_empty() || t.title.len() > 300 {
        return Err(ApiError::bad("a task needs a title (up to 300 characters)"));
    }
    if t.body.len() > 64 * 1024
        || t.acceptance.len() > 50
        || t.labels.len() > 20
        || t.depends_on.len() > 50
    {
        return Err(ApiError::bad(
            "task too large (body 64 KiB, 50 criteria, 20 labels, 50 dependencies)",
        ));
    }
    if t.acceptance
        .iter()
        .chain(&t.labels)
        .any(|x| x.trim().is_empty() || x.len() > 500)
    {
        return Err(ApiError::bad("criteria and labels are 1 to 500 characters"));
    }
    Ok(())
}

pub(crate) async fn create(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<TaskBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s
        .access(&headers, &owner, &name, Action::WriteTasks)
        .await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    validate(&body)?;
    let number =
        s.db.create_task(&NewTask {
            repo_id: &repo.id,
            title: body.title.trim(),
            body: &body.body,
            acceptance: &body.acceptance,
            labels: &body.labels,
            parent: body.parent,
            depends_on: &body.depends_on,
            created_by: &who.actor(),
        })
        .await?;
    s.db.add_event(
        &repo.id,
        &who.actor(),
        "task",
        &json!({ "task": number, "state": "open", "title": body.title.trim() }),
    )
    .await?;
    s.notify(&repo, "task", json!({ "id": number, "state": "open" }));
    Ok(axum::Json(json!(s.db.task(&repo.id, number).await?)))
}

async fn task_json(
    s: &crate::server::Server,
    repo_id: &str,
    number: i64,
) -> ApiResult<axum::Json<Value>> {
    let task =
        s.db.task(repo_id, number)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("task #{number} not found")))?;
    Ok(axum::Json(json!(task)))
}

pub(crate) async fn show(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    task_json(&s, &repo.id, number).await
}

pub(crate) async fn edit(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
    axum::Json(body): axum::Json<TaskBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s
        .access(&headers, &owner, &name, Action::WriteTasks)
        .await?;
    validate(&body)?;
    s.db.edit_task(
        &repo.id,
        number,
        body.title.trim(),
        &body.body,
        &body.acceptance,
        &body.labels,
    )
    .await?;
    s.notify(&repo, "task", json!({ "id": number }));
    task_json(&s, &repo.id, number).await
}

#[derive(Deserialize)]
pub(crate) struct ClaimBody {
    /// How long to hold the task (seconds, 60 to 86400; default 30 minutes).
    /// Claiming again renews it.
    for_secs: Option<i64>,
}

pub(crate) async fn claim(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
    axum::Json(body): axum::Json<ClaimBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s
        .access(&headers, &owner, &name, Action::WorkOnTasks)
        .await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    let secs = body
        .for_secs
        .unwrap_or(1800)
        .clamp(MIN_LEASE_SECS, MAX_LEASE_SECS);
    let task =
        s.db.task(&repo.id, number)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("task #{number} not found")))?;
    if !task.ready && task.state == "open" {
        return Err(ApiError::conflict(format!(
            "task #{number} waits for {}",
            task.depends_on
                .iter()
                .map(|d| format!("#{d}"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if !s
        .db
        .claim_task(&repo.id, number, &who.actor(), now_ms() + secs * 1000)
        .await?
    {
        return Err(ApiError::conflict(format!(
            "task #{number} is {}{}",
            task.state,
            task.claimed_by
                .map(|c| format!(" by {c}"))
                .unwrap_or_default()
        )));
    }
    s.notify(
        &repo,
        "task",
        json!({ "id": number, "state": "claimed", "by": who.actor() }),
    );
    task_json(&s, &repo.id, number).await
}

pub(crate) async fn release(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s
        .access(&headers, &owner, &name, Action::WorkOnTasks)
        .await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    if !s.db.release_task(&repo.id, number, &who.actor()).await? {
        return Err(ApiError::conflict(format!(
            "task #{number} is not claimed by you"
        )));
    }
    s.notify(&repo, "task", json!({ "id": number, "state": "open" }));
    task_json(&s, &repo.id, number).await
}

#[derive(Deserialize)]
pub(crate) struct CloseBody {
    /// `done` (default) or `cancelled`.
    state: Option<String>,
}

pub(crate) async fn close(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
    axum::Json(body): axum::Json<CloseBody>,
) -> ApiResult<axum::Json<Value>> {
    let state = body.state.unwrap_or_else(|| "done".to_owned());
    if state != "done" && state != "cancelled" {
        return Err(ApiError::bad("state is `done` or `cancelled`"));
    }
    let action = if state == "done" {
        Action::WorkOnTasks
    } else {
        Action::WriteTasks
    };
    let (repo, who) = s.access(&headers, &owner, &name, action).await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    if !s
        .db
        .close_task(&repo.id, number, &state, &who.actor())
        .await?
    {
        return Err(ApiError::conflict(format!(
            "task #{number} is already closed"
        )));
    }
    s.db.add_event(
        &repo.id,
        &who.actor(),
        "task",
        &json!({ "task": number, "state": state }),
    )
    .await?;
    s.notify(&repo, "task", json!({ "id": number, "state": state }));
    task_json(&s, &repo.id, number).await
}

pub(crate) async fn reopen(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s
        .access(&headers, &owner, &name, Action::WriteTasks)
        .await?;
    if !s.db.reopen_task(&repo.id, number).await? {
        return Err(ApiError::conflict(format!("task #{number} is not closed")));
    }
    s.notify(&repo, "task", json!({ "id": number, "state": "open" }));
    task_json(&s, &repo.id, number).await
}
