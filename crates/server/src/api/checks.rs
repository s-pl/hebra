//! Check runs as people see them: per commit, with logs; approve waiting
//! runs, re-run, cancel.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use hebra_core::ObjectId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{CheckRun, Repo};
use crate::error::{ApiError, ApiResult};
use crate::identity::Action;
use crate::server::Server;

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    /// A commit id or bookmark; recent runs of the repository when absent.
    commit: Option<String>,
}

pub(crate) async fn list(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    Query(q): Query<ListQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let runs = match q.commit {
        Some(rev) => {
            let commit = s.resolve(&repo, &rev).await?;
            s.db.runs_for_commit(&repo.id, &commit.hex()).await?
        }
        None => s.db.recent_runs(&repo.id, 100).await?,
    };
    Ok(axum::Json(json!({ "checks": runs })))
}

async fn run_in(s: &Server, repo: &Repo, id: &str) -> ApiResult<CheckRun> {
    s.db.run(id)
        .await?
        .filter(|r| r.repo_id == repo.id)
        .ok_or_else(|| ApiError::not_found("check run not found"))
}

pub(crate) async fn show(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    Ok(axum::Json(json!(run_in(&s, &repo, &id).await?)))
}

#[derive(Deserialize)]
pub(crate) struct LogQuery {
    /// Chunks after this sequence number (to follow a running log).
    #[serde(default)]
    after: i64,
}

pub(crate) async fn log(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
    Query(q): Query<LogQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let run = run_in(&s, &repo, &id).await?;
    Ok(axum::Json(
        json!({ "state": run.state, "chunks": s.db.log(&run.id, q.after).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct CommitBody {
    commit: String,
    name: Option<String>,
}

/// Lets runs waiting on a contribution start (maintainers).
pub(crate) async fn approve(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<CommitBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s
        .access(&headers, &owner, &name, Action::ApproveChecks)
        .await?;
    let commit = ObjectId::parse(&body.commit)?;
    let n = s.db.approve_runs(&repo.id, &commit.hex()).await?;
    s.notify(
        &repo,
        "check",
        json!({ "commit": commit.hex(), "approved": n }),
    );
    Ok(axum::Json(json!({ "approved": n })))
}

/// Runs a check again on a commit (all checks when no name is given).
pub(crate) async fn rerun(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<CommitBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Review).await?;
    let commit = ObjectId::parse(&body.commit)?;
    let previous = s.db.runs_for_commit(&repo.id, &commit.hex()).await?;
    let cancelled =
        s.db.supersede_runs(&repo.id, &commit.hex(), body.name.as_deref())
            .await?;
    let review = match previous.iter().find_map(|r| r.review_id.clone()) {
        Some(review_id) => {
            s.db.reviews(&repo.id, "all")
                .await?
                .into_iter()
                .find(|r| r.id == review_id)
        }
        None => None,
    };
    let bookmark = review
        .as_ref()
        .map_or_else(|| "rerun".to_owned(), |r| r.source.clone());
    let created = s
        .trigger_checks(&repo, commit, &bookmark, review.as_ref(), false)
        .await?;
    Ok(axum::Json(
        json!({ "cancelled": cancelled, "queued": created }),
    ))
}

pub(crate) async fn cancel(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, id)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Review).await?;
    let run = run_in(&s, &repo, &id).await?;
    let by = who.map(|w| w.actor()).unwrap_or_default();
    if !s
        .db
        .finish_run(
            &run.id,
            None,
            "cancelled",
            None,
            &format!("cancelled by {by}"),
            0,
        )
        .await?
    {
        return Err(ApiError::conflict("the run already finished"));
    }
    s.notify(
        &repo,
        "check",
        json!({ "id": run.id, "state": "cancelled" }),
    );
    Ok(axum::Json(json!({ "cancelled": run.id })))
}
