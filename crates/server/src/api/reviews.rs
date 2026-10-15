//! Reviews: open, list, show (mergeability, diff, comments, approvals,
//! checks), comment, approve, merge, close.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use hebra_core::ObjectId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::bookmarks::changed_paths;
use crate::api::history::commit_json;
use crate::api::{St, blocking};
use crate::config::Edition;
use crate::db::{NewComment, NewReview, Repo, RepoRole, Review};
use crate::error::{ApiError, ApiResult};
use crate::identity::{Action, Principal, contrib_prefix};
use crate::merging;
use crate::names::{glob_match, valid_bookmark};
use crate::server::Server;

async fn review_or_404(s: &Server, repo: &Repo, number: i64) -> ApiResult<Review> {
    s.db.review(&repo.id, number)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("review #{number} not found")))
}

async fn repo_or_404(s: &Server, owner: &str, name: &str) -> ApiResult<Arc<Repo>> {
    let full_name = format!("{owner}/{name}");
    s.repo(&full_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repository `{full_name}` not found")))
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
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
    if !["open", "queued", "merged", "closed", "all"].contains(&state.as_str()) {
        return Err(ApiError::bad(
            "state is open, queued, merged, closed or all",
        ));
    }
    Ok(axum::Json(
        json!({ "reviews": s.db.reviews(&repo.id, &state).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct OpenReview {
    source: String,
    target: Option<String>,
    title: String,
    #[serde(default)]
    body: String,
}

pub(crate) async fn open(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<OpenReview>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Review).await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    let target = body.target.unwrap_or_else(|| "main".to_owned());
    let title = body.title.trim();
    if title.is_empty() || title.len() > 300 || body.body.len() > 64 * 1024 {
        return Err(ApiError::bad(
            "a review needs a title (up to 300 characters) and a body up to 64 KiB",
        ));
    }
    if !valid_bookmark(&body.source) || !valid_bookmark(&target) || body.source == target {
        return Err(ApiError::bad(
            "source and target are two different, valid bookmarks",
        ));
    }
    // Contributors without write access propose from their own namespace.
    let contribution = s.role(Some(&who), &repo).await? < Some(RepoRole::Write);
    let prefix = contrib_prefix(&who.accountable().handle);
    if contribution && !body.source.starts_with(&prefix) {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("push to `{prefix}…` and open the review from there"),
        ));
    }
    let source = s
        .bookmark_fresh(&repo, &body.source)
        .await?
        .ok_or_else(|| {
            ApiError::bad(format!(
                "bookmark `{}` does not exist on the server; push it first",
                body.source
            ))
        })?;
    if s.bookmark_fresh(&repo, &target).await?.is_none() {
        return Err(ApiError::bad(format!(
            "target bookmark `{target}` does not exist"
        )));
    }
    if let Some(number) =
        s.db.open_review_for(&repo.id, &body.source, &target)
            .await?
    {
        return show_review(&s, &repo, Some(&who), number).await;
    }
    let author = who.actor();
    let number =
        s.db.insert_review(&NewReview {
            repo_id: &repo.id,
            source: &body.source,
            target: &target,
            title,
            body: &body.body,
            author: &author,
            author_id: Some(&who.account.id),
        })
        .await?;
    s.db.add_event(
        &repo.id,
        &author,
        "review",
        &json!({ "review": number, "state": "open", "title": title }),
    )
    .await?;
    let review = review_or_404(&s, &repo, number).await?;
    s.trigger_checks(&repo, source, &review.source, Some(&review), contribution)
        .await?;
    s.notify(&repo, "review", json!({ "id": number, "state": "open" }));
    show_review(&s, &repo, Some(&who), number).await
}

/// What stands between a review and merging: approvals and checks.
async fn requirements(
    s: &Server,
    repo: &Repo,
    review: &Review,
    source: ObjectId,
    target: ObjectId,
) -> ApiResult<Value> {
    let policy = s.policy(repo, Some(target)).await?;
    let rules = policy.for_bookmark(&review.target);
    let store = s.store(repo);
    let changed = blocking(move || {
        let base = merging::merge_base(&store, target, source)?;
        changed_paths(&store, base, Some(source))
    })
    .await?;
    let path_approvals = policy
        .paths
        .iter()
        .filter(|p| changed.iter().any(|c| glob_match(&p.pattern, c)))
        .filter_map(|p| p.required_approvals)
        .max()
        .unwrap_or(0);
    let needed = rules.required_approvals.max(path_approvals);
    let approvals = s.db.approvals(&review.id).await?;
    let head = source.hex();
    let approved = approvals
        .iter()
        .filter(|a| {
            a.state == "approved"
                && a.commit == head
                && Some(&a.account_id) != review.author_id.as_ref()
        })
        .count();
    let changes_requested: Vec<&str> = approvals
        .iter()
        .filter(|a| a.state == "changes_requested")
        .map(|a| a.handle.as_str())
        .collect();
    let (failing, runs) = s.required_checks(repo, &rules, source).await?;
    let ready = approved >= needed as usize && changes_requested.is_empty() && failing.is_empty();
    Ok(json!({
        "approvals_needed": needed,
        "approvals": approved,
        "changes_requested": changes_requested,
        "checks_failing": failing,
        "checks": runs,
        "merge": rules.merge,
        "ready": ready,
    }))
}

async fn show_review(
    s: &Server,
    repo: &Repo,
    who: Option<&Principal>,
    number: i64,
) -> ApiResult<axum::Json<Value>> {
    let review = review_or_404(s, repo, number).await?;
    let mut out = json!(review);
    out["comments"] = json!(s.db.comments(&review.id).await?);
    out["approvals"] = json!(s.db.approvals(&review.id).await?);
    out["role"] = json!(s.role(who, repo).await?);
    let source = s.bookmark_fresh(repo, &review.source).await?;
    let target = s.bookmark_fresh(repo, &review.target).await?;
    if let (Some(source), Some(target)) = (source, target) {
        let store = s.store(repo);
        let (plan, files, commits) = blocking(move || {
            let plan = merging::plan(&store, target, source)?;
            let files = merging::changes(&store, plan.base, source)?;
            let commits: Vec<Value> = merging::commits(&store, target, source, 50)?
                .iter()
                .map(|(id, c)| commit_json(id, c))
                .collect();
            Ok((plan, files, commits))
        })
        .await?;
        out["source_commit"] = json!(source.hex());
        out["target_commit"] = json!(target.hex());
        out["mergeability"] = json!(plan.mergeability);
        out["files"] = json!(files);
        out["commits"] = json!(commits);
        if review.state == "open" || review.state == "queued" {
            out["requirements"] = requirements(s, repo, &review, source, target).await?;
        }
    }
    Ok(axum::Json(out))
}

pub(crate) async fn show(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Read).await?;
    show_review(&s, &repo, who.as_deref(), number).await
}

#[derive(Deserialize)]
pub(crate) struct CommentBody {
    body: String,
    path: Option<String>,
    line: Option<i64>,
    side: Option<String>,
    reply_to: Option<String>,
}

pub(crate) async fn comment(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
    axum::Json(body): axum::Json<CommentBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Comment).await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    let review = review_or_404(&s, &repo, number).await?;
    let text = body.body.trim();
    if text.is_empty() || text.len() > 64 * 1024 {
        return Err(ApiError::bad("a comment has 1 byte to 64 KiB of text"));
    }
    if body
        .side
        .as_deref()
        .is_some_and(|side| side != "old" && side != "new")
    {
        return Err(ApiError::bad("side is `old` or `new`"));
    }
    let head = s
        .bookmark_fresh(&repo, &review.source)
        .await?
        .map(|h| h.hex());
    let author = who.actor();
    let id =
        s.db.add_comment(
            &review.id,
            &NewComment {
                author: &author,
                author_id: Some(&who.account.id),
                path: body.path.as_deref(),
                line: body.line,
                side: body.side.as_deref(),
                commit: head.as_deref(),
                body: text,
                reply_to: body.reply_to.as_deref(),
            },
        )
        .await?;
    s.db.touch_review(&review.id).await?;
    s.notify(&repo, "review", json!({ "id": number, "comment": id }));
    Ok(axum::Json(json!({ "id": id })))
}

#[derive(Deserialize)]
pub(crate) struct ResolveBody {
    #[serde(default = "yes")]
    resolved: bool,
}

fn yes() -> bool {
    true
}

pub(crate) async fn resolve(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number, comment)): UrlPath<(String, String, i64, String)>,
    axum::Json(body): axum::Json<ResolveBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Review).await?;
    let review = review_or_404(&s, &repo, number).await?;
    if !s
        .db
        .resolve_comment(&review.id, &comment, body.resolved)
        .await?
    {
        return Err(ApiError::not_found("comment not found"));
    }
    s.notify(&repo, "review", json!({ "id": number, "comment": comment }));
    Ok(axum::Json(
        json!({ "id": comment, "resolved": body.resolved }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct ApproveBody {
    /// `approved` (default) or `changes_requested`.
    state: Option<String>,
    /// Optional note, added as a comment.
    body: Option<String>,
}

pub(crate) async fn approve(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
    axum::Json(body): axum::Json<ApproveBody>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Approve).await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first"))?;
    let review = review_or_404(&s, &repo, number).await?;
    if review.author_id.as_deref() == Some(who.account.id.as_str()) {
        return Err(ApiError::forbidden(
            "forbidden",
            "you cannot approve your own review",
        ));
    }
    if review.state != "open" && review.state != "queued" {
        return Err(ApiError::bad(format!(
            "review #{number} is {}",
            review.state
        )));
    }
    let state = body.state.unwrap_or_else(|| "approved".to_owned());
    if state != "approved" && state != "changes_requested" {
        return Err(ApiError::bad("state is `approved` or `changes_requested`"));
    }
    let head = s
        .bookmark_fresh(&repo, &review.source)
        .await?
        .ok_or_else(|| ApiError::bad(format!("`{}` no longer exists", review.source)))?;
    s.db.set_approval(&review.id, &who.account.id, &state, &head.hex())
        .await?;
    if let Some(note) = body
        .body
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        s.db.add_comment(
            &review.id,
            &NewComment {
                author: &who.actor(),
                author_id: Some(&who.account.id),
                path: None,
                line: None,
                side: None,
                commit: Some(&head.hex()),
                body: note,
                reply_to: None,
            },
        )
        .await?;
    }
    s.notify(&repo, "review", json!({ "id": number, "approval": state }));
    show_review(&s, &repo, Some(&who), number).await
}

/// Task numbers named in commits' provenance (`#12`, `12`, `T-12`).
fn task_numbers(commits: &[(ObjectId, hebra_core::model::Commit)]) -> Vec<i64> {
    let mut out: Vec<i64> = commits
        .iter()
        .filter_map(|(_, c)| c.provenance.task.as_deref())
        .filter_map(|t| {
            let t = t.trim();
            let digits = t
                .strip_prefix('#')
                .or_else(|| t.strip_prefix("T-"))
                .unwrap_or(t);
            digits.parse::<i64>().ok()
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

pub(crate) async fn merge(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let repo = repo_or_404(&s, &owner, &name).await?;
    // Reading first, so private repositories stay invisible to strangers.
    s.authorize(Some(&who), &repo, Action::Read).await?;
    let review = review_or_404(&s, &repo, number).await?;
    s.authorize(Some(&who), &repo, Action::Merge(&review.target))
        .await?;
    if review.state != "open" && review.state != "queued" {
        return Err(ApiError::bad(format!(
            "review #{number} is {}",
            review.state
        )));
    }
    let source = s
        .bookmark_fresh(&repo, &review.source)
        .await?
        .ok_or_else(|| ApiError::bad(format!("`{}` no longer exists", review.source)))?;
    let target = s
        .bookmark_fresh(&repo, &review.target)
        .await?
        .ok_or_else(|| ApiError::bad(format!("`{}` no longer exists", review.target)))?;
    let req = requirements(&s, &repo, &review, source, target).await?;
    let changes_requested = req["changes_requested"]
        .as_array()
        .is_some_and(|a| !a.is_empty());
    if req["merge"] == "queue" && s.edition == Edition::Cloud {
        // The queue runs the required checks on the merged result; people
        // still have to approve first.
        if req["approvals"].as_u64() < req["approvals_needed"].as_u64() || changes_requested {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "not_ready",
                format!(
                    "review #{number} needs {} approvals (has {}){}",
                    req["approvals_needed"],
                    req["approvals"],
                    if changes_requested {
                        " and has changes requested"
                    } else {
                        ""
                    }
                ),
            ));
        }
        s.enqueue(&repo, &review).await?;
        return show_review(&s, &repo, Some(&who), number).await;
    }
    if req["ready"] != true {
        let mut reasons = vec![format!(
            "{} of {} approvals",
            req["approvals"], req["approvals_needed"]
        )];
        if req["changes_requested"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            reasons.push(format!("changes requested by {}", req["changes_requested"]));
        }
        if req["checks_failing"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            reasons.push(format!(
                "required checks not passing: {}",
                req["checks_failing"]
            ));
        }
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "not_ready",
            format!("review #{number} is not ready: {}", reasons.join("; ")),
        ));
    }
    let store = s.store(&repo);
    let approver = who.as_actor();
    let (src, tgt, title) = (
        review.source.clone(),
        review.target.clone(),
        review.title.clone(),
    );
    // The merge commit (if one is needed) is written first; it only becomes
    // visible when the bookmark moves.
    let (merged, commits) = blocking(move || {
        let plan = merging::plan(&store, target, source)?;
        let commits = merging::commits(&store, target, source, 500)?;
        let merged = match (&plan.mergeability, &plan.tree) {
            (merging::Mergeability::UpToDate, _) => target,
            (merging::Mergeability::FastForward, _) => source,
            (merging::Mergeability::Clean, Some(tree)) => {
                merging::write_merge(&store, target, source, tree, &title, &src, &tgt, &approver)?
            }
            (merging::Mergeability::Conflicts { paths }, _) => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflicts",
                    format!(
                        "merging would conflict in {}; run `hb merge {src}@origin`, resolve, and push",
                        paths.join(", ")
                    ),
                ));
            }
            (merging::Mergeability::Clean, None) => return Err(ApiError::bad("nothing to merge")),
        };
        Ok((merged, commits))
    })
    .await?;
    if merged != target {
        s.move_bookmark(
            &repo,
            &who,
            &review.target,
            Some(target.hex()),
            Some(merged.hex()),
        )
        .await?;
    }
    if !s
        .db
        .finish_review(&review.id, "merged", &who.actor(), Some(&merged.hex()))
        .await?
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "stale",
            format!("review #{number} was closed meanwhile"),
        ));
    }
    s.metrics.merges.fetch_add(1, Ordering::Relaxed);
    s.db.log_event(
        &repo.id,
        "merge",
        &json!({ "review": number, "commit": merged.hex(), "by": who.actor() }),
    )
    .await?;
    for task in task_numbers(&commits) {
        if s.db
            .close_task(&repo.id, task, "done", &format!("review #{number}"))
            .await?
        {
            s.notify(&repo, "task", json!({ "id": task, "state": "done" }));
        }
    }
    s.db.add_event(
        &repo.id,
        &who.actor(),
        "review",
        &json!({ "review": number, "state": "merged" }),
    )
    .await?;
    s.notify(&repo, "review", json!({ "id": number, "state": "merged" }));
    tokio::spawn(crate::checks::trigger_for_push(
        Arc::clone(&s),
        Arc::clone(&repo),
        merged,
        review.target.clone(),
    ));
    show_review(&s, &repo, Some(&who), number).await
}

pub(crate) async fn close(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, number)): UrlPath<(String, String, i64)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let repo = repo_or_404(&s, &owner, &name).await?;
    s.authorize(Some(&who), &repo, Action::Read).await?;
    let review = review_or_404(&s, &repo, number).await?;
    // Authors may withdraw their own review; otherwise it takes write access.
    let action = if review.author_id.as_deref() == Some(who.account.id.as_str()) {
        Action::Comment
    } else {
        Action::Review
    };
    s.authorize(Some(&who), &repo, action).await?;
    if !s
        .db
        .finish_review(&review.id, "closed", &who.actor(), None)
        .await?
    {
        return Err(ApiError::bad(format!(
            "review #{number} is already {}",
            review.state
        )));
    }
    s.db.add_event(
        &repo.id,
        &who.actor(),
        "review",
        &json!({ "review": number, "state": "closed" }),
    )
    .await?;
    s.notify(&repo, "review", json!({ "id": number, "state": "closed" }));
    show_review(&s, &repo, Some(&who), number).await
}

/// The merge queue of a repository: waiting reviews and recent batches.
pub(crate) async fn queue(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    Ok(axum::Json(json!({
        "queued": s.db.reviews(&repo.id, "queued").await?,
        "batches": s.db.batches(&repo.id, 20).await?,
    })))
}

impl Server {
    /// A bookmark moved: open reviews from it get checks for the new head.
    pub(crate) async fn retrigger_reviews(
        &self,
        repo: &Repo,
        source: &str,
        head: ObjectId,
    ) -> ApiResult<()> {
        let reviews = self.db.reviews(&repo.id, "open").await?;
        for review in reviews.iter().filter(|r| r.source == source) {
            let contribution = review.source.starts_with("contrib/");
            self.trigger_checks(repo, head, source, Some(review), contribution)
                .await?;
        }
        Ok(())
    }
}
