//! Environments and their deployments.

use serde::Serialize;
use sqlx::Row;

use super::{Db, json_list, new_id, now_ms, rfc3339};
use crate::error::ApiResult;

#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    pub name: String,
    pub required_approvals: i64,
    pub branches: Vec<String>,
    pub runner_labels: Vec<String>,
    pub url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Deployment {
    pub id: String,
    pub environment: String,
    pub run_id: String,
    pub check: String,
    pub commit: String,
    pub bookmark: Option<String>,
    pub state: String,
    pub approved_by: Vec<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

pub struct EnvironmentRules<'a> {
    pub required_approvals: i64,
    pub branches: &'a [String],
    pub runner_labels: &'a [String],
    pub url: Option<&'a str>,
}

fn env_row(r: &sqlx::any::AnyRow) -> Result<Environment, sqlx::Error> {
    Ok(Environment {
        name: r.try_get("name")?,
        required_approvals: r.try_get("required_approvals")?,
        branches: json_list(r, "branches")?,
        runner_labels: json_list(r, "runner_labels")?,
        url: r.try_get("url")?,
        created_at: rfc3339(r.try_get("created_at")?),
        updated_at: rfc3339(r.try_get("updated_at")?),
    })
}

impl Db {
    pub async fn environments(&self, repo_id: &str) -> ApiResult<Vec<Environment>> {
        let rows = sqlx::query(
            "SELECT name, required_approvals, branches, runner_labels, url, created_at, updated_at
             FROM environments WHERE repo_id = $1 ORDER BY name",
        )
        .bind(repo_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(env_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn environment(&self, repo_id: &str, name: &str) -> ApiResult<Option<Environment>> {
        let row = sqlx::query(
            "SELECT name, required_approvals, branches, runner_labels, url, created_at, updated_at
             FROM environments WHERE repo_id = $1 AND name = $2",
        )
        .bind(repo_id)
        .bind(name)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(env_row).transpose()?)
    }

    pub async fn put_environment(
        &self,
        repo_id: &str,
        name: &str,
        rules: &EnvironmentRules<'_>,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO environments (repo_id, name, required_approvals, branches, runner_labels, url, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
             ON CONFLICT (repo_id, name) DO UPDATE SET
                required_approvals = excluded.required_approvals, branches = excluded.branches,
                runner_labels = excluded.runner_labels, url = excluded.url, updated_at = excluded.updated_at",
        )
        .bind(repo_id)
        .bind(name)
        .bind(rules.required_approvals)
        .bind(serde_json::to_string(rules.branches)?)
        .bind(serde_json::to_string(rules.runner_labels)?)
        .bind(rules.url)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_environment(&self, repo_id: &str, name: &str) -> ApiResult<bool> {
        Ok(
            sqlx::query("DELETE FROM environments WHERE repo_id = $1 AND name = $2")
                .bind(repo_id)
                .bind(name)
                .execute(self.pool())
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn record_deployment(
        &self,
        repo_id: &str,
        environment: &str,
        run_id: &str,
        commit: &str,
        bookmark: Option<&str>,
    ) -> ApiResult<String> {
        let id = new_id("dep");
        sqlx::query(
            "INSERT INTO deployments (id, repo_id, environment, run_id, commit_id, bookmark, approved_by, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, '[]', $7)",
        )
        .bind(&id)
        .bind(repo_id)
        .bind(environment)
        .bind(run_id)
        .bind(commit)
        .bind(bookmark)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        sqlx::query("UPDATE check_runs SET environment = $1 WHERE id = $2")
            .bind(environment)
            .bind(run_id)
            .execute(self.pool())
            .await?;
        Ok(id)
    }

    pub async fn deployments(
        &self,
        repo_id: &str,
        environment: Option<&str>,
        limit: i64,
    ) -> ApiResult<Vec<Deployment>> {
        let rows = sqlx::query(
            "SELECT d.id, d.environment, d.run_id, c.name, d.commit_id, d.bookmark, c.state, d.approved_by,
                    c.summary, d.created_at, c.finished_at
             FROM deployments d JOIN check_runs c ON c.id = d.run_id
             WHERE d.repo_id = $1 AND ($2 IS NULL OR d.environment = $2)
             ORDER BY d.created_at DESC LIMIT $3",
        )
        .bind(repo_id)
        .bind(environment)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(Deployment {
                    id: r.try_get("id")?,
                    environment: r.try_get("environment")?,
                    run_id: r.try_get("run_id")?,
                    check: r.try_get("name")?,
                    commit: r.try_get("commit_id")?,
                    bookmark: r.try_get("bookmark")?,
                    state: r.try_get("state")?,
                    approved_by: json_list(r, "approved_by")?,
                    summary: r.try_get("summary")?,
                    created_at: rfc3339(r.try_get("created_at")?),
                    finished_at: r.try_get::<Option<i64>, _>("finished_at")?.map(rfc3339),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Adds an approval to a run's deployment; returns everyone who has
    /// approved it so far.
    pub async fn approve_deployment(&self, run_id: &str, by: &str) -> ApiResult<Vec<String>> {
        let current: Option<String> =
            sqlx::query_scalar("SELECT approved_by FROM deployments WHERE run_id = $1")
                .bind(run_id)
                .fetch_optional(self.pool())
                .await?;
        let mut list: Vec<String> = current
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();
        if !list.iter().any(|b| b == by) {
            list.push(by.to_owned());
        }
        sqlx::query("UPDATE deployments SET approved_by = $1 WHERE run_id = $2")
            .bind(serde_json::to_string(&list)?)
            .bind(run_id)
            .execute(self.pool())
            .await?;
        Ok(list)
    }

    /// Lets one waiting run start.
    pub async fn release_run(&self, run_id: &str) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE check_runs SET state = 'queued' WHERE id = $1 AND state = 'waiting'",
        )
        .bind(run_id)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    /// The environment a run deploys to.
    pub async fn run_environment(&self, run_id: &str) -> ApiResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT environment FROM check_runs WHERE id = $1")
                .bind(run_id)
                .fetch_optional(self.pool())
                .await?
                .flatten(),
        )
    }
}
