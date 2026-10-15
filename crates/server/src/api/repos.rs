//! Repositories: listing, creation, settings, access, forks and events.

use std::collections::HashSet;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{AccountKind, NewRepo, OrgRole, Repo, RepoRole};
use crate::error::{ApiError, ApiResult};
use crate::identity::{Action, Principal};
use crate::names::{valid_bookmark, valid_handle, valid_repo_name};
use crate::server::Server;

fn who_actor(who: &Option<std::sync::Arc<Principal>>) -> String {
    who.as_ref().map(|w| w.actor()).unwrap_or_default()
}

/// The transparency log of a repository, with the server's signature over
/// its head (`hb verify-log` checks both).
pub(crate) async fn transparency(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    Query(q): Query<TransparencyQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let entries =
        s.db.log_entries(&repo.id, q.after.unwrap_or(0), 1000)
            .await?;
    let head = s.db.log_head(&repo.id).await?.map(|(seq, hash)| {
        let statement = hebra_core::transparency::head_statement(&repo.full_name, seq, &hash);
        json!({ "seq": seq, "hash": hash, "signature": s.authority.sign(statement.as_bytes()) })
    });
    Ok(axum::Json(json!({
        "repo": repo.full_name,
        "entries": entries,
        "head": head,
        "public_key": s.authority.public_hex(),
    })))
}

#[derive(Deserialize)]
pub(crate) struct TransparencyQuery {
    after: Option<i64>,
}

/// What a caller can see: public repositories, those of owners they belong
/// to, and those granted to them or their teams.
struct Visibility {
    admin: bool,
    owners: HashSet<String>,
    granted: HashSet<String>,
}

impl Visibility {
    async fn of(s: &Server, who: Option<&Principal>) -> ApiResult<Self> {
        let Some(who) = who else {
            return Ok(Self {
                admin: false,
                owners: HashSet::new(),
                granted: HashSet::new(),
            });
        };
        let person = who.accountable();
        let mut owners: HashSet<String> = HashSet::from([person.id.clone()]);
        owners.extend(
            s.db.orgs_of(&person.id)
                .await?
                .into_iter()
                .filter(|m| m.role != OrgRole::Billing)
                .map(|m| m.account.id),
        );
        let mut principals = s.db.teams_of(&person.id).await?;
        principals.push(person.id.clone());
        principals.push(who.account.id.clone());
        Ok(Self {
            admin: who.account.site_admin,
            owners,
            granted: s.db.granted_repos(&principals).await?.into_iter().collect(),
        })
    }

    fn sees(&self, repo: &Repo) -> bool {
        repo.public
            || self.admin
            || self.owners.contains(&repo.owner_id)
            || self.granted.contains(&repo.id)
    }
}

/// Public repositories, and private ones the caller can see.
pub(crate) async fn list(State(s): St, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let who = s.authenticate(&headers).await?;
    let vis = Visibility::of(&s, who.as_deref()).await?;
    let repos: Vec<_> =
        s.db.repo_summaries(None)
            .await?
            .into_iter()
            .filter(|r| vis.sees(&r.repo))
            .collect();
    Ok(axum::Json(json!({ "repos": repos })))
}

pub(crate) async fn list_owned(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.authenticate(&headers).await?;
    let owner =
        s.db.account_by_handle(&handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    let vis = Visibility::of(&s, who.as_deref()).await?;
    let repos: Vec<_> =
        s.db.repo_summaries(Some(&owner.id))
            .await?
            .into_iter()
            .filter(|r| vis.sees(&r.repo))
            .collect();
    Ok(axum::Json(json!({ "repos": repos })))
}

#[derive(Deserialize)]
pub(crate) struct CreateRepo {
    /// Account or organization handle; defaults to the caller.
    owner: Option<String>,
    name: String,
    #[serde(default)]
    public: bool,
    #[serde(default)]
    description: String,
    protect: Option<Vec<String>>,
}

/// Whether `who` may create repositories under `owner`.
async fn may_create_under(
    s: &Server,
    who: &Principal,
    owner: &crate::db::Account,
) -> ApiResult<bool> {
    if who.is_agent() {
        return Ok(false);
    }
    if owner.id == who.account.id || who.account.site_admin {
        return Ok(true);
    }
    Ok(owner.kind == AccountKind::Org
        && matches!(
            s.db.org_role(&owner.id, &who.account.id).await?,
            Some(OrgRole::Owner | OrgRole::Admin | OrgRole::Member)
        ))
}

pub(crate) async fn create(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CreateRepo>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner_handle = body
        .owner
        .clone()
        .unwrap_or_else(|| who.account.handle.clone());
    let owner =
        s.db.account_by_handle(&owner_handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{owner_handle}` not found")))?;
    if !may_create_under(&s, &who, &owner).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you cannot create repositories under `{owner_handle}`"),
        ));
    }
    if !valid_repo_name(&body.name) {
        return Err(ApiError::bad(
            "a repository name has letters, digits, `-`, `_` or `.`, and does not end in .git",
        ));
    }
    let protect = body.protect.unwrap_or_else(|| vec!["main".to_owned()]);
    if protect
        .iter()
        .any(|p| !valid_bookmark(&p.replace('*', "x")))
    {
        return Err(ApiError::bad("invalid protected bookmark pattern"));
    }
    if !body.public {
        s.check_private_repo_quota(&owner).await?;
    }
    let full_name = format!("{}/{}", owner.handle, body.name);
    let repo =
        s.db.create_repo(
            &owner.handle,
            &NewRepo {
                owner_id: &owner.id,
                name: &body.name,
                public: body.public,
                description: body.description.trim(),
                protect: &protect,
                forked_from: None,
            },
        )
        .await?;
    s.objects
        .create(&repo.id)
        .map_err(|e| ApiError::internal("objects", e))?;
    s.caches.repos.remove(&full_name);
    Ok(axum::Json(json!(repo)))
}

pub(crate) async fn info(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Read).await?;
    let role = s.role(who.as_deref(), &repo).await?;
    Ok(axum::Json(json!({
        "name": repo.full_name,
        "repo": *repo,
        "role": role,
        "bookmarks": *s.bookmarks(&repo).await?,
        "protected": *s.protected(&repo.id).await?,
        "open_reviews": s.db.count_open_reviews(&repo.id).await?,
    })))
}

#[derive(Deserialize)]
pub(crate) struct UpdateRepo {
    description: Option<String>,
    public: Option<bool>,
    open_contributions: Option<bool>,
    protect: Option<Vec<String>>,
}

pub(crate) async fn update(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<UpdateRepo>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let public = body.public.unwrap_or(repo.public);
    if repo.public && !public {
        let owner =
            s.db.account(&repo.owner_id)
                .await?
                .ok_or_else(|| ApiError::internal("database", "owner missing"))?;
        s.check_private_repo_quota(&owner).await?;
    }
    s.db.update_repo(
        &repo.id,
        body.description
            .as_deref()
            .unwrap_or(&repo.description)
            .trim(),
        public,
        body.open_contributions.unwrap_or(repo.open_contributions),
    )
    .await?;
    if let Some(protect) = body.protect {
        if protect
            .iter()
            .any(|p| !valid_bookmark(&p.replace('*', "x")))
        {
            return Err(ApiError::bad("invalid protected bookmark pattern"));
        }
        s.db.set_protected(&repo.id, &protect).await?;
    }
    s.forget_repo(&repo);
    let fresh = s.db.repo_by_id(&repo.id).await?;
    s.db.log_event(
        &repo.id,
        "settings",
        &json!({ "repo": fresh, "by": who_actor(&who) }),
    )
    .await?;
    Ok(axum::Json(json!(fresh)))
}

pub(crate) async fn remove(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    if who.as_ref().is_some_and(|w| w.is_agent()) {
        return Err(ApiError::forbidden(
            "needs_human",
            "only people delete repositories",
        ));
    }
    s.db.delete_repo(&repo.id).await?;
    s.forget_repo(&repo);
    // Forks keep reading these objects; only delete them when nothing does.
    let forks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM repos WHERE lineage LIKE $1")
        .bind(format!("%\"{}\"%", repo.id))
        .fetch_one(s.db.pool())
        .await?;
    if forks == 0 {
        let objects_repo = repo.id.clone();
        let server = std::sync::Arc::clone(&s);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = server.objects.delete(&objects_repo) {
                tracing::warn!(repo = objects_repo, error = %e, "could not delete objects");
            }
        });
    }
    Ok(axum::Json(json!({ "deleted": repo.full_name })))
}

// --------------------------------------------------------------- access

pub(crate) async fn access(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let mut out = Vec::new();
    for (principal, role) in s.db.access_list(&repo.id).await? {
        let who = if principal.starts_with("team_") {
            json!({ "team": principal })
        } else {
            match s.db.account(&principal).await? {
                Some(a) => json!({ "account": a.handle, "kind": a.kind }),
                None => continue,
            }
        };
        out.push(json!({ "principal": who, "role": role }));
    }
    Ok(axum::Json(json!({ "access": out })))
}

/// `ana`, an agent handle, or `team:<handle>` (a team of the owning
/// organization).
async fn principal_id(s: &Server, repo: &Repo, principal: &str) -> ApiResult<String> {
    if let Some(team) = principal.strip_prefix("team:") {
        return s.db.team_id(&repo.owner_id, team).await?.ok_or_else(|| {
            ApiError::not_found(format!(
                "team `{team}` not found in the owning organization"
            ))
        });
    }
    if !valid_handle(principal) {
        return Err(ApiError::bad(format!("invalid name `{principal}`")));
    }
    Ok(s.db
        .account_by_handle(principal)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("`{principal}` not found")))?
        .id)
}

#[derive(Deserialize)]
pub(crate) struct Grant {
    role: String,
}

pub(crate) async fn grant(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, principal)): UrlPath<(String, String, String)>,
    axum::Json(body): axum::Json<Grant>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let role = RepoRole::parse(&body.role)?;
    let id = principal_id(&s, &repo, &principal).await?;
    s.db.set_access(&repo.id, &id, Some(role)).await?;
    s.db.log_event(
        &repo.id,
        "access",
        &json!({ "principal": principal, "role": role, "by": who_actor(&who) }),
    )
    .await?;
    s.forget_repo(&repo);
    Ok(axum::Json(json!({ "principal": principal, "role": role })))
}

pub(crate) async fn revoke(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name, principal)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, who) = s.access(&headers, &owner, &name, Action::Admin).await?;
    let id = principal_id(&s, &repo, &principal).await?;
    s.db.set_access(&repo.id, &id, None).await?;
    s.db.log_event(
        &repo.id,
        "access",
        &json!({ "principal": principal, "role": null, "by": who_actor(&who) }),
    )
    .await?;
    s.forget_repo(&repo);
    Ok(axum::Json(json!({ "principal": principal, "role": null })))
}

// ---------------------------------------------------------------- forks

#[derive(Deserialize)]
pub(crate) struct Fork {
    owner: Option<String>,
    name: Option<String>,
}

/// An independent copy under another owner. Instant: the fork reads the
/// original's objects and keeps only what is written afterwards.
pub(crate) async fn fork(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<Fork>,
) -> ApiResult<axum::Json<Value>> {
    let (source, who) = s.access(&headers, &owner, &name, Action::Read).await?;
    let who = who.ok_or_else(|| ApiError::unauthorized("sign in first (`hb login`)"))?;
    let owner_handle = body.owner.unwrap_or_else(|| who.account.handle.clone());
    let target_owner =
        s.db.account_by_handle(&owner_handle)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("`{owner_handle}` not found")))?;
    if !may_create_under(&s, &who, &target_owner).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you cannot create repositories under `{owner_handle}`"),
        ));
    }
    let fork_name = body.name.unwrap_or_else(|| source.name.clone());
    if !valid_repo_name(&fork_name) {
        return Err(ApiError::bad("invalid repository name"));
    }
    if !source.public {
        s.check_private_repo_quota(&target_owner).await?;
    }
    let fork =
        s.db.create_repo(
            &target_owner.handle,
            &NewRepo {
                owner_id: &target_owner.id,
                name: &fork_name,
                public: source.public,
                description: &source.description,
                protect: &s.protected(&source.id).await?,
                forked_from: Some(&source),
            },
        )
        .await?;
    s.objects
        .create(&fork.id)
        .map_err(|e| ApiError::internal("objects", e))?;
    for (bookmark, target) in s.bookmarks(&source).await?.iter() {
        s.db.cas_bookmark(&crate::db::Move {
            repo_id: &fork.id,
            name: bookmark,
            old: None,
            new: Some(target),
            actor: &who.actor(),
        })
        .await?
        .map_err(|_| ApiError::internal("fork", "bookmark appeared while forking"))?;
    }
    s.db.add_event(
        &fork.id,
        &who.actor(),
        "fork",
        &json!({ "from": source.full_name }),
    )
    .await?;
    Ok(axum::Json(json!(fork)))
}

// --------------------------------------------------------------- events

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    limit: Option<usize>,
}

pub(crate) async fn events(
    State(s): St,
    headers: HeaderMap,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<axum::Json<Value>> {
    let (repo, _) = s.access(&headers, &owner, &name, Action::Read).await?;
    let limit = i64::try_from(q.limit.unwrap_or(30).min(200)).unwrap_or(30);
    Ok(axum::Json(
        json!({ "events": s.db.events(&repo.id, limit).await? }),
    ))
}
