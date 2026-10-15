//! People, organizations, teams, agents and mandates.

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{Account, AccountKind, OrgRole};
use crate::error::{ApiError, ApiResult};
use crate::identity::Principal;
use crate::names::valid_handle;
use crate::server::Server;

/// Longest a mandate issued through the API may live.
const MAX_MANDATE_SECS: i64 = 365 * 24 * 3600;

async fn account(s: &Server, handle: &str) -> ApiResult<Account> {
    s.db.account_by_handle(handle)
        .await?
        .filter(|a| !a.disabled)
        .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))
}

async fn org(s: &Server, handle: &str) -> ApiResult<Account> {
    let a = account(s, handle).await?;
    if a.kind != AccountKind::Org {
        return Err(ApiError::bad(format!("`{handle}` is not an organization")));
    }
    Ok(a)
}

/// The account `who` manages, or a 403.
async fn managed(s: &Server, who: &Principal, handle: &str) -> ApiResult<Account> {
    let a = account(s, handle).await?;
    if !s.manages(who, &a, false).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you do not manage `{handle}`"),
        ));
    }
    Ok(a)
}

/// Issues a root mandate for `account` and records it.
async fn issue(
    s: &Server,
    account: &Account,
    label: &str,
    ttl_secs: Option<i64>,
    by: &str,
) -> ApiResult<(String, String)> {
    let ttl = ttl_secs.map(|t| t.clamp(60, MAX_MANDATE_SECS));
    let expires = ttl.map(|t| jiff::Timestamp::now().as_second() + t);
    let (mandate, root) = s
        .authority
        .issue(&account.id, expires)
        .map_err(|e| ApiError::internal("mandate", e))?;
    s.db.record_mandate(
        &root,
        &account.id,
        label,
        Some(by),
        expires.map(|e| e * 1000),
    )
    .await?;
    Ok((mandate, root))
}

// ----------------------------------------------------------------- me

pub(crate) async fn me(State(s): St, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let orgs = s.db.orgs_of(&who.account.id).await?;
    Ok(axum::Json(json!({
        "account": who.account,
        "owner": who.owner.clone().map(Account::public),
        "agent": who.is_agent(),
        "delegation": who.delegation(),
        "display": who.actor(),
        "orgs": orgs,
        // Kept for older clients: name/kind/admin at the top level.
        "name": who.account.handle,
        "kind": if who.is_agent() { "agent" } else { "human" },
        "admin": who.account.site_admin,
    })))
}

pub(crate) async fn my_mandates(State(s): St, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    Ok(axum::Json(
        json!({ "mandates": s.db.mandates_of(&who.account.id).await? }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct NewMandate {
    label: String,
    /// Seconds; none for no expiry.
    ttl_secs: Option<i64>,
}

/// A new root mandate for yourself (people only, signed in on the web or
/// with an undelegated mandate).
pub(crate) async fn issue_mandate(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<NewMandate>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    if who.is_agent() {
        return Err(ApiError::forbidden(
            "needs_human",
            "agents cannot mint mandates; narrow the one you have with `hb mandate grant`",
        ));
    }
    let label = body.label.trim();
    if label.is_empty() || label.len() > 100 {
        return Err(ApiError::bad(
            "a mandate needs a label (up to 100 characters)",
        ));
    }
    let (mandate, id) = issue(&s, &who.account, label, body.ttl_secs, &who.account.handle).await?;
    Ok(axum::Json(json!({ "id": id, "mandate": mandate })))
}

pub(crate) async fn revoke_mandate(
    State(s): St,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    if !s
        .db
        .revoke_mandate(&id, &who.account.id, "revoked by owner")
        .await?
    {
        return Err(ApiError::not_found("no such mandate of yours"));
    }
    s.caches.principals.clear();
    Ok(axum::Json(json!({ "revoked": id })))
}

// ------------------------------------------------------------ accounts

pub(crate) async fn profile(
    State(s): St,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let a = account(&s, &handle).await?;
    let owner = match &a.owner_id {
        Some(id) => s.db.account(id).await?.map(Account::public),
        None => None,
    };
    Ok(axum::Json(json!({ "account": a.public(), "owner": owner })))
}

#[derive(Deserialize)]
pub(crate) struct NewOrg {
    handle: String,
    name: String,
}

pub(crate) async fn create_org(
    State(s): St,
    headers: HeaderMap,
    axum::Json(body): axum::Json<NewOrg>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    if who.is_agent() {
        return Err(ApiError::forbidden(
            "needs_human",
            "only people create organizations",
        ));
    }
    if !valid_handle(&body.handle) {
        return Err(ApiError::bad(
            "an organization name has 1–39 lowercase letters, digits or hyphens",
        ));
    }
    let name = body.name.trim();
    let org =
        s.db.create_account(
            &body.handle,
            AccountKind::Org,
            if name.is_empty() { &body.handle } else { name },
            None,
            None,
        )
        .await?;
    s.db.set_member(&org.id, &who.account.id, OrgRole::Owner)
        .await?;
    Ok(axum::Json(json!({ "account": org })))
}

pub(crate) async fn members(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = org(&s, &handle).await?;
    if s.db
        .org_role(&org.id, &who.accountable().id)
        .await?
        .is_none()
        && !who.account.site_admin
    {
        return Err(ApiError::not_found(format!("`{handle}` not found")));
    }
    let members: Vec<Value> =
        s.db.members(&org.id)
            .await?
            .into_iter()
            .map(|m| json!({ "account": m.account.public(), "role": m.role }))
            .collect();
    Ok(axum::Json(json!({ "members": members })))
}

#[derive(Deserialize)]
pub(crate) struct RoleBody {
    role: String,
}

pub(crate) async fn set_member(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, member)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<RoleBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = managed(&s, &who, &handle).await?;
    let person = account(&s, &member).await?;
    if person.kind != AccountKind::Human {
        return Err(ApiError::bad(
            "members are people; give agents access to repositories instead",
        ));
    }
    let role = OrgRole::parse(&body.role)?;
    let current = s.db.org_role(&org.id, &person.id).await?;
    if role == OrgRole::Owner || current == Some(OrgRole::Owner) {
        // Only owners make or unmake owners.
        if s.db.org_role(&org.id, &who.account.id).await? != Some(OrgRole::Owner)
            && !who.account.site_admin
        {
            return Err(ApiError::forbidden(
                "forbidden",
                "only owners change owners",
            ));
        }
        if current == Some(OrgRole::Owner)
            && role != OrgRole::Owner
            && s.db.count_owners(&org.id).await? <= 1
        {
            return Err(ApiError::bad("an organization keeps at least one owner"));
        }
    }
    if current.is_none() {
        s.check_people_quota(&org).await?;
    }
    s.db.set_member(&org.id, &person.id, role).await?;
    s.caches.roles.clear();
    Ok(axum::Json(json!({ "member": person.handle, "role": role })))
}

pub(crate) async fn remove_member(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, member)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = org(&s, &handle).await?;
    let person = account(&s, &member).await?;
    // People may leave; admins may remove others.
    if person.id != who.account.id && !s.manages(&who, &org, false).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you do not manage `{handle}`"),
        ));
    }
    if s.db.org_role(&org.id, &person.id).await? == Some(OrgRole::Owner)
        && s.db.count_owners(&org.id).await? <= 1
    {
        return Err(ApiError::bad("an organization keeps at least one owner"));
    }
    s.db.remove_member(&org.id, &person.id).await?;
    s.caches.roles.clear();
    Ok(axum::Json(json!({ "removed": person.handle })))
}

// --------------------------------------------------------------- teams

pub(crate) async fn teams(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = org(&s, &handle).await?;
    if s.db
        .org_role(&org.id, &who.accountable().id)
        .await?
        .is_none()
        && !who.account.site_admin
    {
        return Err(ApiError::not_found(format!("`{handle}` not found")));
    }
    Ok(axum::Json(json!({ "teams": s.db.teams(&org.id).await? })))
}

#[derive(Deserialize)]
pub(crate) struct NewTeam {
    handle: String,
    name: Option<String>,
}

pub(crate) async fn create_team(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<NewTeam>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = managed(&s, &who, &handle).await?;
    if org.kind != AccountKind::Org || !valid_handle(&body.handle) {
        return Err(ApiError::bad(
            "teams belong to organizations and need a valid name",
        ));
    }
    let id =
        s.db.create_team(
            &org.id,
            &body.handle,
            body.name.as_deref().unwrap_or(&body.handle),
        )
        .await?;
    Ok(axum::Json(json!({ "id": id, "handle": body.handle })))
}

async fn team_id(s: &Server, org: &Account, team: &str) -> ApiResult<String> {
    s.db.team_id(&org.id, team)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("team `{team}` not found")))
}

pub(crate) async fn delete_team(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, team)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = managed(&s, &who, &handle).await?;
    s.db.delete_team(&team_id(&s, &org, &team).await?).await?;
    s.caches.roles.clear();
    Ok(axum::Json(json!({ "deleted": team })))
}

async fn team_member(
    s: Arc<Server>,
    headers: HeaderMap,
    handle: String,
    team: String,
    member: String,
    add: bool,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let org = managed(&s, &who, &handle).await?;
    let team_id = team_id(&s, &org, &team).await?;
    let person = account(&s, &member).await?;
    if add && s.db.org_role(&org.id, &person.id).await?.is_none() {
        return Err(ApiError::bad(format!(
            "`{member}` is not a member of `{handle}`"
        )));
    }
    s.db.set_team_member(&team_id, &person.id, add).await?;
    s.caches.roles.clear();
    Ok(axum::Json(
        json!({ "team": team, "member": person.handle, "in_team": add }),
    ))
}

pub(crate) async fn add_team_member(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, team, member)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    team_member(s, headers, handle, team, member, true).await
}

pub(crate) async fn remove_team_member(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, team, member)): UrlPath<(String, String, String)>,
) -> ApiResult<axum::Json<Value>> {
    team_member(s, headers, handle, team, member, false).await
}

// -------------------------------------------------------------- agents

pub(crate) async fn agents(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let agents: Vec<Account> =
        s.db.owned_accounts(&owner.id)
            .await?
            .into_iter()
            .filter(|a| a.kind == AccountKind::Agent)
            .collect();
    Ok(axum::Json(json!({ "agents": agents })))
}

#[derive(Deserialize)]
pub(crate) struct NewAgent {
    handle: String,
    name: Option<String>,
    /// Seconds the first mandate lasts (default 90 days).
    ttl_secs: Option<i64>,
}

/// An agent account owned by a person or organization, with its first
/// mandate. Agents of people act with that person's access; agents of
/// organizations get only what repositories grant them.
pub(crate) async fn create_agent(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<NewAgent>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    if !valid_handle(&body.handle) {
        return Err(ApiError::bad(
            "an agent name has 1–39 lowercase letters, digits or hyphens",
        ));
    }
    let agent =
        s.db.create_account(
            &body.handle,
            AccountKind::Agent,
            body.name.as_deref().unwrap_or(&body.handle),
            None,
            Some(&owner.id),
        )
        .await?;
    let (mandate, id) = issue(
        &s,
        &agent,
        "first mandate",
        Some(body.ttl_secs.unwrap_or(90 * 24 * 3600)),
        &who.account.handle,
    )
    .await?;
    Ok(axum::Json(
        json!({ "account": agent, "mandate": mandate, "mandate_id": id }),
    ))
}

async fn owned_agent(s: &Server, owner: &Account, handle: &str) -> ApiResult<Account> {
    let a = account(s, handle).await?;
    if a.owner_id.as_deref() != Some(owner.id.as_str()) || a.kind != AccountKind::Agent {
        return Err(ApiError::not_found(format!("agent `{handle}` not found")));
    }
    Ok(a)
}

pub(crate) async fn agent_mandate(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, agent)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<NewMandate>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let agent = owned_agent(&s, &owner, &agent).await?;
    let (mandate, id) = issue(
        &s,
        &agent,
        body.label.trim(),
        body.ttl_secs,
        &who.account.handle,
    )
    .await?;
    Ok(axum::Json(json!({ "id": id, "mandate": mandate })))
}

/// Disables an agent: every mandate it holds stops working.
pub(crate) async fn disable_agent(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, agent)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let agent = owned_agent(&s, &owner, &agent).await?;
    s.db.disable_account(&agent.id).await?;
    s.caches.principals.clear();
    Ok(axum::Json(json!({ "disabled": agent.handle })))
}
