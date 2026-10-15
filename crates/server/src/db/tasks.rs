//! Tasks: agent-friendly issues with acceptance criteria, leases instead of
//! assignees, and dependencies.

use serde::Serialize;
use sqlx::Row;
use sqlx::any::AnyRow;

use super::{Db, json_list, new_id, now_ms, rfc3339};
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    #[serde(skip)]
    pub id: String,
    #[serde(rename = "id")]
    pub number: i64,
    pub title: String,
    pub body: String,
    pub acceptance: Vec<String>,
    pub labels: Vec<String>,
    /// `open`, `claimed` (lease alive), `done` or `cancelled`. A claim whose
    /// lease ran out reads as `open`.
    pub state: String,
    pub parent: Option<i64>,
    pub depends_on: Vec<i64>,
    /// Every dependency is done, so the task can start.
    pub ready: bool,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub claimed_by: Option<String>,
    pub lease_until: Option<String>,
    pub closed_by: Option<String>,
}

pub struct NewTask<'a> {
    pub repo_id: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub acceptance: &'a [String],
    pub labels: &'a [String],
    pub parent: Option<i64>,
    pub depends_on: &'a [i64],
    pub created_by: &'a str,
}

macro_rules! task_select {
    ($tail:literal) => {
        concat!(
            "SELECT t.id, t.number, t.title, t.body, t.acceptance, t.labels, t.state, p.number AS parent,
                    t.created_by, t.created_at, t.updated_at, t.claimed_by, t.lease_until, t.closed_by
             FROM tasks t LEFT JOIN tasks p ON p.id = t.parent_id ",
            $tail
        )
    };
}

fn task_row(r: &AnyRow, now: i64) -> Result<Task, sqlx::Error> {
    let mut state: String = r.try_get("state")?;
    let lease: Option<i64> = r.try_get("lease_until")?;
    let mut claimed_by: Option<String> = r.try_get("claimed_by")?;
    if state == "claimed" && lease.is_none_or(|l| l <= now) {
        state = "open".to_owned();
        claimed_by = None;
    }
    Ok(Task {
        id: r.try_get("id")?,
        number: r.try_get("number")?,
        title: r.try_get("title")?,
        body: r.try_get("body")?,
        acceptance: json_list(r, "acceptance")?,
        labels: json_list(r, "labels")?,
        lease_until: if claimed_by.is_some() {
            lease.map(rfc3339)
        } else {
            None
        },
        state,
        parent: r.try_get("parent")?,
        depends_on: Vec::new(),
        ready: true,
        created_by: r.try_get("created_by")?,
        created_at: rfc3339(r.try_get("created_at")?),
        updated_at: rfc3339(r.try_get("updated_at")?),
        claimed_by,
        closed_by: r.try_get("closed_by")?,
    })
}

impl Db {
    async fn task_id(&self, repo_id: &str, number: i64) -> ApiResult<String> {
        sqlx::query_scalar("SELECT id FROM tasks WHERE repo_id = $1 AND number = $2")
            .bind(repo_id)
            .bind(number)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| ApiError::not_found(format!("task #{number} not found")))
    }

    /// Fills in dependencies and readiness.
    async fn with_deps(&self, repo_id: &str, mut tasks: Vec<Task>) -> ApiResult<Vec<Task>> {
        let deps: Vec<(String, i64, String)> = sqlx::query_as(
            "SELECT d.task_id, t.number, t.state FROM task_deps d JOIN tasks t ON t.id = d.depends_on
             WHERE t.repo_id = $1",
        )
        .bind(repo_id)
        .fetch_all(self.pool())
        .await?;
        for task in &mut tasks {
            for (task_id, number, state) in &deps {
                if *task_id == task.id {
                    task.depends_on.push(*number);
                    if state != "done" {
                        task.ready = false;
                    }
                }
            }
        }
        Ok(tasks)
    }

    pub async fn create_task(&self, t: &NewTask<'_>) -> ApiResult<i64> {
        let parent_id = match t.parent {
            Some(n) => Some(self.task_id(t.repo_id, n).await?),
            None => None,
        };
        let mut dep_ids = Vec::new();
        for n in t.depends_on {
            dep_ids.push(self.task_id(t.repo_id, *n).await?);
        }
        let number = self.next_number(&format!("task:{}", t.repo_id)).await?;
        let id = new_id("task");
        let now = now_ms();
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO tasks (id, repo_id, number, title, body, acceptance, labels, state, parent_id,
                                created_by, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'open', $8, $9, $10, $11)",
        )
        .bind(&id)
        .bind(t.repo_id)
        .bind(number)
        .bind(t.title)
        .bind(t.body)
        .bind(serde_json::to_string(t.acceptance)?)
        .bind(serde_json::to_string(t.labels)?)
        .bind(parent_id)
        .bind(t.created_by)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for dep in &dep_ids {
            sqlx::query("INSERT INTO task_deps (task_id, depends_on) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(&id)
                .bind(dep)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(number)
    }

    pub async fn task(&self, repo_id: &str, number: i64) -> ApiResult<Option<Task>> {
        let row = sqlx::query(task_select!("WHERE t.repo_id = $1 AND t.number = $2"))
            .bind(repo_id)
            .bind(number)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else { return Ok(None) };
        let task = task_row(&row, now_ms())?;
        Ok(self.with_deps(repo_id, vec![task]).await?.pop())
    }

    /// Tasks of a repository; `state` is a task state, `ready` (open with
    /// every dependency done) or `all`.
    pub async fn tasks(&self, repo_id: &str, state: &str) -> ApiResult<Vec<Task>> {
        let rows = sqlx::query(task_select!(
            "WHERE t.repo_id = $1 ORDER BY t.number DESC LIMIT 500"
        ))
        .bind(repo_id)
        .fetch_all(self.pool())
        .await?;
        let now = now_ms();
        let tasks = rows
            .iter()
            .map(|r| task_row(r, now))
            .collect::<Result<Vec<_>, _>>()?;
        let tasks = self.with_deps(repo_id, tasks).await?;
        Ok(tasks
            .into_iter()
            .filter(|t| match state {
                "all" => true,
                "ready" => t.state == "open" && t.ready,
                s => t.state == s,
            })
            .collect())
    }

    /// Claims a task for `who` until `lease_until`, if it is open (or its
    /// lease ran out, or `who` already holds it: that renews it).
    pub async fn claim_task(
        &self,
        repo_id: &str,
        number: i64,
        who: &str,
        lease_until: i64,
    ) -> ApiResult<bool> {
        let now = now_ms();
        Ok(sqlx::query(
            "UPDATE tasks SET state = 'claimed', claimed_by = $1, lease_until = $2, updated_at = $3
             WHERE repo_id = $4 AND number = $5
               AND (state = 'open' OR (state = 'claimed' AND (lease_until <= $6 OR claimed_by = $7)))",
        )
        .bind(who)
        .bind(lease_until)
        .bind(now)
        .bind(repo_id)
        .bind(number)
        .bind(now)
        .bind(who)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    /// Gives a claimed task back (only its holder can).
    pub async fn release_task(&self, repo_id: &str, number: i64, who: &str) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE tasks SET state = 'open', claimed_by = NULL, lease_until = NULL, updated_at = $1
             WHERE repo_id = $2 AND number = $3 AND state = 'claimed' AND claimed_by = $4",
        )
        .bind(now_ms())
        .bind(repo_id)
        .bind(number)
        .bind(who)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    /// Marks a task `done` or `cancelled`. False if it was already closed.
    pub async fn close_task(
        &self,
        repo_id: &str,
        number: i64,
        state: &str,
        by: &str,
    ) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE tasks SET state = $1, closed_by = $2, claimed_by = NULL, lease_until = NULL, updated_at = $3
             WHERE repo_id = $4 AND number = $5 AND state IN ('open', 'claimed')",
        )
        .bind(state)
        .bind(by)
        .bind(now_ms())
        .bind(repo_id)
        .bind(number)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn reopen_task(&self, repo_id: &str, number: i64) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE tasks SET state = 'open', closed_by = NULL, updated_at = $1
             WHERE repo_id = $2 AND number = $3 AND state IN ('done', 'cancelled')",
        )
        .bind(now_ms())
        .bind(repo_id)
        .bind(number)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn edit_task(
        &self,
        repo_id: &str,
        number: i64,
        title: &str,
        body: &str,
        acceptance: &[String],
        labels: &[String],
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE tasks SET title = $1, body = $2, acceptance = $3, labels = $4, updated_at = $5
             WHERE repo_id = $6 AND number = $7",
        )
        .bind(title)
        .bind(body)
        .bind(serde_json::to_string(acceptance)?)
        .bind(serde_json::to_string(labels)?)
        .bind(now_ms())
        .bind(repo_id)
        .bind(number)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}
