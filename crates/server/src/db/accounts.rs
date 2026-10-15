//! Accounts (people, agents, organizations, runners), organization
//! membership and teams.

use serde::Serialize;
use sqlx::Row;
use sqlx::any::AnyRow;

use super::{Db, flag, new_id, now_ms, rfc3339};
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Human,
    Agent,
    Org,
    Runner,
}

impl AccountKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Org => "org",
            Self::Runner => "runner",
        }
    }

    fn parse(s: &str) -> Result<Self, sqlx::Error> {
        Ok(match s {
            "human" => Self::Human,
            "agent" => Self::Agent,
            "org" => Self::Org,
            "runner" => Self::Runner,
            other => {
                return Err(sqlx::Error::Decode(
                    format!("account kind `{other}`").into(),
                ));
            }
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: String,
    pub handle: String,
    pub kind: AccountKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    /// For agents and runners: the person or organization they belong to.
    pub owner_id: Option<String>,
    pub site_admin: bool,
    pub created_at: String,
    pub disabled: bool,
}

impl Account {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: r.try_get("id")?,
            handle: r.try_get("handle")?,
            kind: AccountKind::parse(&r.try_get::<String, _>("kind")?)?,
            name: r.try_get("name")?,
            email: r.try_get("email")?,
            avatar_url: r.try_get("avatar_url")?,
            owner_id: r.try_get("owner_id")?,
            site_admin: flag(r, "site_admin")?,
            created_at: rfc3339(r.try_get("created_at")?),
            disabled: r.try_get::<Option<i64>, _>("disabled_at")?.is_some(),
        })
    }

    /// The account without private fields, for other people to see.
    #[must_use]
    pub fn public(mut self) -> Self {
        self.email = None;
        self
    }
}

macro_rules! account_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, handle, kind, name, email, avatar_url, owner_id, site_admin, created_at, disabled_at
             FROM accounts ",
            $tail
        )
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    Billing,
    Member,
    Admin,
    Owner,
}

impl OrgRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Billing => "billing",
            Self::Member => "member",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> ApiResult<Self> {
        Ok(match s {
            "billing" => Self::Billing,
            "member" => Self::Member,
            "admin" => Self::Admin,
            "owner" => Self::Owner,
            other => {
                return Err(ApiError::bad(format!(
                    "unknown organization role `{other}`"
                )));
            }
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub account: Account,
    pub role: OrgRole,
}

#[derive(Debug, Clone, Serialize)]
pub struct Team {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub members: Vec<String>,
}

impl Db {
    /// Creates an account. Fails with a conflict if the handle is taken.
    pub async fn create_account(
        &self,
        handle: &str,
        kind: AccountKind,
        name: &str,
        email: Option<&str>,
        owner_id: Option<&str>,
    ) -> ApiResult<Account> {
        let id = new_id(match kind {
            AccountKind::Org => "org",
            AccountKind::Runner => "run",
            AccountKind::Agent => "agt",
            AccountKind::Human => "usr",
        });
        let inserted = sqlx::query(
            "INSERT INTO accounts (id, handle, kind, name, email, owner_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
        )
        .bind(&id)
        .bind(handle)
        .bind(kind.as_str())
        .bind(name)
        .bind(email)
        .bind(owner_id)
        .bind(now_ms())
        .execute(self.pool())
        .await?
        .rows_affected();
        if inserted == 0 {
            return Err(ApiError::conflict(format!("the name `{handle}` is taken")));
        }
        self.account(&id)
            .await?
            .ok_or_else(|| ApiError::internal("database", "account vanished after insert"))
    }

    pub async fn account(&self, id: &str) -> ApiResult<Option<Account>> {
        let row = sqlx::query(account_select!("WHERE id = $1"))
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(Account::from_row).transpose()?)
    }

    pub async fn account_by_handle(&self, handle: &str) -> ApiResult<Option<Account>> {
        let row = sqlx::query(account_select!("WHERE handle = $1"))
            .bind(handle)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(Account::from_row).transpose()?)
    }

    /// The account linked to an external identity (`github`, user id).
    pub async fn account_by_identity(
        &self,
        provider: &str,
        subject: &str,
    ) -> ApiResult<Option<Account>> {
        let row = sqlx::query(account_select!(
            "WHERE id = (SELECT account_id FROM identities WHERE provider = $1 AND subject = $2)"
        ))
        .bind(provider)
        .bind(subject)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(Account::from_row).transpose()?)
    }

    pub async fn link_identity(
        &self,
        provider: &str,
        subject: &str,
        account_id: &str,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO identities (provider, subject, account_id, created_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(provider)
        .bind(subject)
        .bind(account_id)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn update_profile(
        &self,
        id: &str,
        name: &str,
        avatar_url: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query("UPDATE accounts SET name = $1, avatar_url = $2 WHERE id = $3")
            .bind(name)
            .bind(avatar_url)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn set_site_admin(&self, id: &str, admin: bool) -> ApiResult<()> {
        sqlx::query("UPDATE accounts SET site_admin = $1 WHERE id = $2")
            .bind(i64::from(admin))
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn disable_account(&self, id: &str) -> ApiResult<()> {
        sqlx::query("UPDATE accounts SET disabled_at = $1 WHERE id = $2 AND disabled_at IS NULL")
            .bind(now_ms())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Agents and runners that belong to `owner_id`.
    pub async fn owned_accounts(&self, owner_id: &str) -> ApiResult<Vec<Account>> {
        let rows = sqlx::query(account_select!("WHERE owner_id = $1 ORDER BY handle"))
            .bind(owner_id)
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .map(Account::from_row)
            .collect::<Result<_, _>>()?)
    }

    pub async fn count_humans(&self) -> ApiResult<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE kind = 'human'")
                .fetch_one(self.pool())
                .await?,
        )
    }

    // ------------------------------------------------------- organizations

    pub async fn org_role(&self, org_id: &str, account_id: &str) -> ApiResult<Option<OrgRole>> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM org_members WHERE org_id = $1 AND account_id = $2",
        )
        .bind(org_id)
        .bind(account_id)
        .fetch_optional(self.pool())
        .await?;
        role.as_deref().map(OrgRole::parse).transpose()
    }

    /// Adds a member or changes their role.
    pub async fn set_member(&self, org_id: &str, account_id: &str, role: OrgRole) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO org_members (org_id, account_id, role, created_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (org_id, account_id) DO UPDATE SET role = excluded.role",
        )
        .bind(org_id)
        .bind(account_id)
        .bind(role.as_str())
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn remove_member(&self, org_id: &str, account_id: &str) -> ApiResult<bool> {
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "DELETE FROM team_members WHERE account_id = $1
               AND team_id IN (SELECT id FROM teams WHERE org_id = $2)",
        )
        .bind(account_id)
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
        let n = sqlx::query("DELETE FROM org_members WHERE org_id = $1 AND account_id = $2")
            .bind(org_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(n == 1)
    }

    pub async fn count_owners(&self, org_id: &str) -> ApiResult<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM org_members WHERE org_id = $1 AND role = 'owner'",
        )
        .bind(org_id)
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn count_members(&self, org_id: &str) -> ApiResult<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM org_members WHERE org_id = $1")
                .bind(org_id)
                .fetch_one(self.pool())
                .await?,
        )
    }

    pub async fn members(&self, org_id: &str) -> ApiResult<Vec<Member>> {
        let rows = sqlx::query(
            "SELECT a.id, a.handle, a.kind, a.name, a.email, a.avatar_url, a.owner_id, a.site_admin,
                    a.created_at, a.disabled_at, m.role
             FROM org_members m JOIN accounts a ON a.id = m.account_id
             WHERE m.org_id = $1 ORDER BY a.handle",
        )
        .bind(org_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(Member {
                    account: Account::from_row(r)?,
                    role: OrgRole::parse(&r.try_get::<String, _>("role")?)?,
                })
            })
            .collect()
    }

    /// Organizations `account_id` belongs to, with its role in each.
    pub async fn orgs_of(&self, account_id: &str) -> ApiResult<Vec<Member>> {
        let rows = sqlx::query(
            "SELECT a.id, a.handle, a.kind, a.name, a.email, a.avatar_url, a.owner_id, a.site_admin,
                    a.created_at, a.disabled_at, m.role
             FROM org_members m JOIN accounts a ON a.id = m.org_id
             WHERE m.account_id = $1 ORDER BY a.handle",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(Member {
                    account: Account::from_row(r)?.public(),
                    role: OrgRole::parse(&r.try_get::<String, _>("role")?)?,
                })
            })
            .collect()
    }

    // ----------------------------------------------------------------- teams

    pub async fn create_team(&self, org_id: &str, handle: &str, name: &str) -> ApiResult<String> {
        let id = new_id("team");
        let n = sqlx::query(
            "INSERT INTO teams (id, org_id, handle, name, created_at) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(&id)
        .bind(org_id)
        .bind(handle)
        .bind(name)
        .bind(now_ms())
        .execute(self.pool())
        .await?
        .rows_affected();
        if n == 0 {
            return Err(ApiError::conflict(format!(
                "team `{handle}` already exists"
            )));
        }
        Ok(id)
    }

    pub async fn team_id(&self, org_id: &str, handle: &str) -> ApiResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT id FROM teams WHERE org_id = $1 AND handle = $2")
                .bind(org_id)
                .bind(handle)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn delete_team(&self, team_id: &str) -> ApiResult<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("DELETE FROM repo_access WHERE principal_id = $1")
            .bind(team_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM teams WHERE id = $1")
            .bind(team_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_team_member(
        &self,
        team_id: &str,
        account_id: &str,
        member: bool,
    ) -> ApiResult<()> {
        let sql = if member {
            "INSERT INTO team_members (team_id, account_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        } else {
            "DELETE FROM team_members WHERE team_id = $1 AND account_id = $2"
        };
        sqlx::query(sql)
            .bind(team_id)
            .bind(account_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn teams(&self, org_id: &str) -> ApiResult<Vec<Team>> {
        let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT t.id, t.handle, t.name, a.handle
             FROM teams t
             LEFT JOIN team_members tm ON tm.team_id = t.id
             LEFT JOIN accounts a ON a.id = tm.account_id
             WHERE t.org_id = $1 ORDER BY t.handle, a.handle",
        )
        .bind(org_id)
        .fetch_all(self.pool())
        .await?;
        let mut teams: Vec<Team> = Vec::new();
        for (id, handle, name, member) in rows {
            if teams.last().is_none_or(|t| t.id != id) {
                teams.push(Team {
                    id,
                    handle,
                    name,
                    members: Vec::new(),
                });
            }
            if let (Some(team), Some(member)) = (teams.last_mut(), member) {
                team.members.push(member);
            }
        }
        Ok(teams)
    }

    /// Ids of the teams `account_id` is in.
    pub async fn teams_of(&self, account_id: &str) -> ApiResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT team_id FROM team_members WHERE account_id = $1")
                .bind(account_id)
                .fetch_all(self.pool())
                .await?,
        )
    }
}
