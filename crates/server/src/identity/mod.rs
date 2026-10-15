//! Who is calling and what they may do.
//!
//! Every request carries at most one credential in `Authorization: Bearer`:
//! - `hbs_…`: a web session (a person, full rights of their account);
//! - `hbm_…`: a mandate, possibly attenuated (see `hebra-mandate`).
//!
//! [`Server::authorize`] is the single place that decides whether an action
//! on a repository is allowed. It combines, in order:
//! 1. the account's role in the repository (ownership, organization role,
//!    direct and team grants, public read);
//! 2. the mandate's own checks (repositories, operations, bookmarks, time);
//! 3. the rules for agents (only people approve, and only people move
//!    protected bookmarks) and for open contributions.

pub mod github;
pub mod secret;

use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, header};
use hebra_mandate::{Delegation, Verified, op};

use crate::db::{Account, AccountKind, OrgRole, Repo, RepoRole};
use crate::error::{ApiError, ApiResult};
use crate::names::glob_match;
use crate::server::Server;

/// How long a replica trusts an authentication or role lookup before asking
/// the database again: revocations reach every replica within this time.
pub(crate) const AUTH_TTL: Duration = Duration::from_secs(5);
pub(crate) const NEGATIVE_TTL: Duration = Duration::from_secs(1);

/// An authenticated caller.
pub struct Principal {
    pub account: Account,
    /// For agents and runners: the person or organization they belong to.
    pub owner: Option<Account>,
    /// Set when the caller used a mandate (not a web session).
    pub mandate: Option<Verified>,
}

impl Principal {
    /// Agents, runners, and anyone using a delegated mandate act as agents:
    /// they cannot approve reviews or move protected bookmarks.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        self.account.kind != AccountKind::Human
            || self.mandate.as_ref().is_some_and(|m| m.delegated)
    }

    #[must_use]
    pub fn delegation(&self) -> Option<&Delegation> {
        self.mandate
            .as_ref()
            .filter(|m| m.delegated)
            .map(|m| &m.delegation)
    }

    /// How this caller appears in history and events:
    /// `ana`, `claude-code (agent for ana)`, `ci-bot (agent of acme)`.
    #[must_use]
    pub fn actor(&self) -> String {
        let handle = &self.account.handle;
        if let Some(d) = self.delegation() {
            let agent = d.agent.as_deref().unwrap_or("agent");
            return format!("{agent} (agent for {handle})");
        }
        match (self.account.kind, &self.owner) {
            (AccountKind::Agent | AccountKind::Runner, Some(owner)) => {
                format!("{handle} (agent of {})", owner.handle)
            }
            _ => handle.clone(),
        }
    }

    /// This caller as a commit author.
    #[must_use]
    pub fn as_actor(&self) -> hebra_core::model::Actor {
        use hebra_core::model::{Actor, ActorKind};
        match self.delegation() {
            Some(d) => Actor {
                name: d.agent.clone().unwrap_or_else(|| "agent".to_owned()),
                kind: ActorKind::Agent,
                on_behalf_of: Some(self.account.handle.clone()),
            },
            None if self.account.kind == AccountKind::Human => Actor {
                name: self.account.handle.clone(),
                kind: ActorKind::Human,
                on_behalf_of: None,
            },
            None => Actor {
                name: self.account.handle.clone(),
                kind: ActorKind::Agent,
                on_behalf_of: self.owner.as_ref().map(|o| o.handle.clone()),
            },
        }
    }

    /// The human or organization accountable for this caller.
    #[must_use]
    pub fn accountable(&self) -> &Account {
        self.owner.as_ref().unwrap_or(&self.account)
    }
}

/// Something done to a repository.
#[derive(Debug, Clone, Copy)]
pub enum Action<'a> {
    Read,
    /// Send objects (they become reachable only when a bookmark moves).
    Upload,
    /// Move this bookmark.
    Push(&'a str),
    /// Open a review, or update one.
    Review,
    /// Comment on a review.
    Comment,
    /// Approve or request changes.
    Approve,
    /// Merge a review into this bookmark.
    Merge(&'a str),
    /// Create and edit tasks.
    WriteTasks,
    /// Claim and complete tasks.
    WorkOnTasks,
    /// Let waiting checks run (contributions).
    ApproveChecks,
    /// Settings, access, deletion.
    Admin,
    /// Report check results (runners of the repository's owner).
    Run,
}

impl<'a> Action<'a> {
    fn needs(self) -> RepoRole {
        match self {
            Action::Read | Action::Comment => RepoRole::Read,
            Action::WriteTasks => RepoRole::Triage,
            Action::Upload
            | Action::Push(_)
            | Action::Review
            | Action::Approve
            | Action::Merge(_)
            | Action::WorkOnTasks
            | Action::Run => RepoRole::Write,
            Action::ApproveChecks => RepoRole::Maintain,
            Action::Admin => RepoRole::Admin,
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Action::Read => op::READ,
            Action::Upload | Action::Push(_) => op::PUSH,
            Action::Review | Action::Comment => op::REVIEW,
            Action::Approve | Action::Merge(_) => op::MERGE,
            Action::WriteTasks | Action::WorkOnTasks => op::TASK,
            Action::ApproveChecks | Action::Admin => op::ADMIN,
            Action::Run => op::RUN,
        }
    }

    /// The bookmark the action touches, if any.
    fn bookmark(self) -> Option<&'a str> {
        match self {
            Action::Push(b) | Action::Merge(b) => Some(b),
            _ => None,
        }
    }
}

/// The namespace where `handle` may push to a repository with open
/// contributions.
#[must_use]
pub fn contrib_prefix(handle: &str) -> String {
    format!("contrib/{handle}/")
}

fn credential(headers: &HeaderMap) -> ApiResult<Option<&str>> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::unauthorized("malformed authorization header"))?;
    let token = value.strip_prefix("Bearer ").unwrap_or(value).trim();
    if token.is_empty() || token.len() > 16 * 1024 {
        return Err(ApiError::unauthorized("malformed credential"));
    }
    Ok(Some(token))
}

impl Server {
    /// The caller, if they sent a credential. Unknown, expired or revoked
    /// credentials are an error (never silently anonymous).
    pub(crate) async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> ApiResult<Option<Arc<Principal>>> {
        let Some(token) = credential(headers)? else {
            return Ok(None);
        };
        let key = secret::hash(token);
        if let Some(cached) = self.caches.principals.get(&key) {
            return cached
                .map(Some)
                .ok_or_else(|| ApiError::unauthorized("unknown, expired or revoked credential"));
        }
        let found = self.lookup_principal(token).await?;
        let ttl = if found.is_some() {
            AUTH_TTL
        } else {
            NEGATIVE_TTL
        };
        self.caches.principals.put(&key, found.clone(), ttl);
        found
            .map(Some)
            .ok_or_else(|| ApiError::unauthorized("unknown, expired or revoked credential"))
    }

    async fn lookup_principal(&self, token: &str) -> ApiResult<Option<Arc<Principal>>> {
        let (account_id, mandate) = if token.starts_with(secret::SESSION_PREFIX) {
            match self.db.session_account(&secret::hash(token)).await? {
                Some(id) => (id, None),
                None => return Ok(None),
            }
        } else if token.starts_with(hebra_mandate::PREFIX) {
            let Ok(verified) = self.authority.verify(token) else {
                return Ok(None);
            };
            if self.db.any_revoked(&verified.revocation_ids).await? {
                return Ok(None);
            }
            if let Some(root) = verified.revocation_ids.first() {
                // Best effort: "last used" in the mandate list.
                let _ = self.db.touch_mandate(root).await;
            }
            (verified.account_id.clone(), Some(verified))
        } else {
            return Ok(None);
        };
        let Some(account) = self.db.account(&account_id).await? else {
            return Ok(None);
        };
        if account.disabled {
            return Ok(None);
        }
        let owner = match &account.owner_id {
            Some(id) => match self.db.account(id).await? {
                Some(owner) if !owner.disabled => Some(owner),
                _ => return Ok(None),
            },
            None => None,
        };
        Ok(Some(Arc::new(Principal {
            account,
            owner,
            mandate,
        })))
    }

    pub(crate) async fn require_principal(&self, headers: &HeaderMap) -> ApiResult<Arc<Principal>> {
        self.authenticate(headers)
            .await?
            .ok_or_else(|| ApiError::unauthorized("sign in first (`hb login`)"))
    }

    /// A repository by `owner/name`, cached briefly.
    pub(crate) async fn repo(&self, full_name: &str) -> ApiResult<Option<Arc<Repo>>> {
        if let Some(cached) = self.caches.repos.get(full_name) {
            return Ok(cached);
        }
        let found = self.db.repo(full_name).await?.map(Arc::new);
        let ttl = if found.is_some() {
            AUTH_TTL
        } else {
            NEGATIVE_TTL
        };
        self.caches.repos.put(full_name, found.clone(), ttl);
        Ok(found)
    }

    /// The role of a person in a repository, from ownership, organization
    /// membership and grants (directly or through teams).
    async fn human_role(&self, account: &Account, repo: &Repo) -> ApiResult<Option<RepoRole>> {
        if account.site_admin || repo.owner_id == account.id {
            return Ok(Some(RepoRole::Admin));
        }
        let mut role = match self.db.org_role(&repo.owner_id, &account.id).await? {
            Some(OrgRole::Owner | OrgRole::Admin) => return Ok(Some(RepoRole::Admin)),
            Some(OrgRole::Member) => Some(RepoRole::Read),
            Some(OrgRole::Billing) | None => None,
        };
        let mut principals = self.db.teams_of(&account.id).await?;
        principals.push(account.id.clone());
        role = role.max(self.db.granted_role(&repo.id, &principals).await?);
        Ok(role)
    }

    /// What `who` may do in `repo`, before mandate checks.
    pub(crate) async fn role(
        &self,
        who: Option<&Principal>,
        repo: &Repo,
    ) -> ApiResult<Option<RepoRole>> {
        let public = repo.public.then_some(RepoRole::Read);
        let Some(who) = who else { return Ok(public) };
        let key = format!("{}:{}", who.account.id, repo.id);
        if let Some(cached) = self.caches.roles.get(&key) {
            return Ok(cached.max(public));
        }
        let role = match (who.account.kind, &who.owner) {
            (AccountKind::Human, _) => self.human_role(&who.account, repo).await?,
            // An agent of a person acts with that person's role.
            (AccountKind::Agent, Some(owner)) if owner.kind == AccountKind::Human => {
                self.human_role(owner, repo).await?
            }
            // Runners read the code of their owner's repositories.
            (AccountKind::Runner, Some(owner)) if owner.id == repo.owner_id => Some(RepoRole::Read),
            // Organization agents get what they were granted.
            _ => {
                self.db
                    .granted_role(&repo.id, std::slice::from_ref(&who.account.id))
                    .await?
            }
        };
        self.caches.roles.put(&key, role, AUTH_TTL);
        Ok(role.max(public))
    }

    /// Decides whether `who` may do `action` in `repo`. Returns the caller's
    /// role. Private repositories look absent to those who cannot read them.
    pub(crate) async fn authorize(
        &self,
        who: Option<&Principal>,
        repo: &Repo,
        action: Action<'_>,
    ) -> ApiResult<RepoRole> {
        let role = self.role(who, repo).await?;
        let Some(role) = role else {
            return Err(ApiError::not_found(format!(
                "repository `{}` not found",
                repo.full_name
            )));
        };
        let bookmark = action.bookmark();
        if !matches!(action, Action::Read) && who.is_none() {
            return Err(ApiError::unauthorized("sign in first (`hb login`)"));
        }
        let allowed_by_role = role >= action.needs()
            || match (action, who) {
                // Open contributions: anyone signed in pushes to their own
                // namespace and opens reviews from it.
                (Action::Push(b), Some(p)) => {
                    repo.open_contributions
                        && b.starts_with(&contrib_prefix(&p.accountable().handle))
                }
                (Action::Review | Action::Upload, Some(_)) => repo.open_contributions,
                (Action::Run, Some(p)) => {
                    p.account.kind == AccountKind::Runner
                        && p.owner.as_ref().is_some_and(|o| o.id == repo.owner_id)
                }
                _ => false,
            };
        if !allowed_by_role {
            return Err(ApiError::forbidden(
                "forbidden",
                format!(
                    "you have {} access to `{}`; this needs {}",
                    role.as_str(),
                    repo.full_name,
                    action.needs().as_str()
                ),
            ));
        }
        let Some(who) = who else { return Ok(role) };
        if let Some(mandate) = &who.mandate {
            hebra_mandate::authorize(
                mandate,
                &hebra_mandate::Request {
                    repo: Some(&repo.full_name),
                    operation: action.operation(),
                    bookmark,
                    now: jiff::Timestamp::now().as_second(),
                },
            )
            .map_err(|e| ApiError::forbidden("mandate", e.to_string()))?;
        }
        if who.is_agent() {
            if matches!(action, Action::Approve) {
                return Err(ApiError::forbidden(
                    "needs_human",
                    "only people approve reviews; agents can comment",
                ));
            }
            if let Some(b) = bookmark
                && self
                    .protected(&repo.id)
                    .await?
                    .iter()
                    .any(|p| glob_match(p, b))
            {
                return Err(ApiError::forbidden(
                    "needs_human",
                    format!(
                        "`{b}` is protected: agents push to their own bookmarks and a person moves `{b}`"
                    ),
                ));
            }
        }
        Ok(role)
    }

    /// Looks up `owner/name` and authorizes `action` on it in one step.
    pub(crate) async fn access(
        &self,
        headers: &HeaderMap,
        owner: &str,
        name: &str,
        action: Action<'_>,
    ) -> ApiResult<(Arc<Repo>, Option<Arc<Principal>>)> {
        let who = self.authenticate(headers).await?;
        let full_name = format!("{owner}/{name}");
        let repo = self
            .repo(&full_name)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("repository `{full_name}` not found")))?;
        self.authorize(who.as_deref(), &repo, action).await?;
        Ok((repo, who))
    }

    /// Whether `who` administers the account `owner` (themselves, or an
    /// organization where they are owner or admin; billing role for billing).
    pub(crate) async fn manages(
        &self,
        who: &Principal,
        owner: &Account,
        billing: bool,
    ) -> ApiResult<bool> {
        if who.is_agent() {
            return Ok(false);
        }
        if who.account.site_admin || who.account.id == owner.id {
            return Ok(true);
        }
        if owner.kind != AccountKind::Org {
            return Ok(false);
        }
        Ok(match self.db.org_role(&owner.id, &who.account.id).await? {
            Some(OrgRole::Owner | OrgRole::Admin) => true,
            Some(OrgRole::Billing) => billing,
            _ => false,
        })
    }
}
