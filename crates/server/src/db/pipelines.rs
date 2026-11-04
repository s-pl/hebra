//! What pipelines need beyond the run queue: secrets, artifacts, one-time
//! runner tokens, results to reuse, and run history for Heddle.

use serde::Serialize;
use sqlx::Row;

use super::{Db, json_list, new_id, now_ms, rfc3339};
use crate::db::CheckRun;
use crate::error::ApiResult;

#[derive(Debug, Clone, Serialize)]
pub struct SecretInfo {
    pub name: String,
    pub scope: String,
    pub updated_at: String,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub id: String,
    pub run_id: String,
    pub name: String,
    pub size: i64,
    pub digest: String,
    pub created_at: String,
}

/// A runner token to store (only its hash).
pub struct NewRunnerToken<'a> {
    pub hash: &'a str,
    pub owner_id: &'a str,
    pub labels: &'a [String],
    pub ephemeral: bool,
    pub uses: i64,
    pub expires_at: i64,
    pub by: &'a str,
}

/// What a one-time token lets a machine become.
#[derive(Debug, Clone)]
pub struct RunnerGrant {
    pub owner_id: String,
    pub labels: Vec<String>,
    pub ephemeral: bool,
}

/// One finished run, for duration and failure statistics.
#[derive(Debug, Clone, Serialize)]
pub struct RunStat {
    pub name: String,
    pub state: String,
    pub labels: Vec<String>,
    pub queued_ms: i64,
    pub started_ms: Option<i64>,
    pub finished_ms: Option<i64>,
    pub attempt: i64,
    pub reused: bool,
}

/// A queued run, as an orchestrator sees the queue.
#[derive(Debug, Clone, Serialize)]
pub struct QueuedRun {
    pub id: String,
    pub repo: String,
    pub name: String,
    pub labels: Vec<String>,
    pub queued_at: String,
    pub waiting_ms: i64,
    pub image: Option<String>,
    pub blocked: bool,
}

impl Db {
    /// Forgets artifacts made before `before_ms`; their files go with the
    /// next sweep. Returns how many.
    pub async fn prune_artifacts(&self, before_ms: i64) -> ApiResult<u64> {
        Ok(sqlx::query("DELETE FROM artifacts WHERE created_at < $1")
            .bind(before_ms)
            .execute(self.pool())
            .await?
            .rows_affected())
    }

    /// Forgets webhook deliveries and fleet events older than `before_ms`.
    pub async fn prune_logs(&self, before_ms: i64) -> ApiResult<u64> {
        let deliveries = sqlx::query("DELETE FROM webhook_deliveries WHERE at < $1")
            .bind(before_ms)
            .execute(self.pool())
            .await?
            .rows_affected();
        let events = sqlx::query("DELETE FROM fleet_events WHERE at < $1 AND kind <> 'settings'")
            .bind(before_ms - 60 * 86_400_000)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deliveries + events)
    }

    /// Every stored file something still points at (artifacts, packages,
    /// image layers, release files), as hex digests.
    pub async fn referenced_files(&self) -> ApiResult<std::collections::HashSet<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT digest FROM artifacts
             UNION SELECT digest FROM package_versions
             UNION SELECT digest FROM package_blobs
             UNION SELECT digest FROM release_assets",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(d,)| crate::files::hex_of(&d).map(str::to_owned))
            .collect())
    }

    // --------------------------------------------------------------- secrets

    pub async fn put_secret(
        &self,
        scope: &str,
        owner_id: &str,
        name: &str,
        sealed: &str,
        by: &str,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO secrets (scope, owner_id, name, sealed, created_at, updated_at, updated_by)
             VALUES ($1, $2, $3, $4, $5, $5, $6)
             ON CONFLICT (scope, owner_id, name) DO UPDATE
             SET sealed = excluded.sealed, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        )
        .bind(scope)
        .bind(owner_id)
        .bind(name)
        .bind(sealed)
        .bind(now)
        .bind(by)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn secrets(&self, scope: &str, owner_id: &str) -> ApiResult<Vec<SecretInfo>> {
        let rows = sqlx::query(
            "SELECT name, scope, updated_at, updated_by FROM secrets WHERE scope = $1 AND owner_id = $2 ORDER BY name",
        )
        .bind(scope)
        .bind(owner_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SecretInfo {
                    name: r.try_get("name")?,
                    scope: r.try_get("scope")?,
                    updated_at: rfc3339(r.try_get("updated_at")?),
                    updated_by: r.try_get("updated_by")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    pub async fn delete_secret(&self, scope: &str, owner_id: &str, name: &str) -> ApiResult<bool> {
        Ok(
            sqlx::query("DELETE FROM secrets WHERE scope = $1 AND owner_id = $2 AND name = $3")
                .bind(scope)
                .bind(owner_id)
                .bind(name)
                .execute(self.pool())
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn sealed_secret(
        &self,
        scope: &str,
        owner_id: &str,
        name: &str,
    ) -> ApiResult<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT sealed FROM secrets WHERE scope = $1 AND owner_id = $2 AND name = $3",
        )
        .bind(scope)
        .bind(owner_id)
        .bind(name)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Sealed values for a run, `(name, scope, owner, sealed)`: the
    /// repository's, else its owner's.
    pub async fn sealed_secrets(
        &self,
        repo_id: &str,
        owner_id: &str,
        names: &[String],
    ) -> ApiResult<Vec<(String, String, String, String)>> {
        let mut out = Vec::new();
        for name in names {
            let row: Option<(String, String, String)> = sqlx::query_as(
                "SELECT scope, owner_id, sealed FROM secrets
                 WHERE name = $1 AND ((scope = 'repo' AND owner_id = $2) OR (scope = 'account' AND owner_id = $3))
                 ORDER BY CASE scope WHEN 'repo' THEN 0 ELSE 1 END LIMIT 1",
            )
            .bind(name)
            .bind(repo_id)
            .bind(owner_id)
            .fetch_optional(self.pool())
            .await?;
            if let Some((scope, owner, sealed)) = row {
                out.push((name.clone(), scope, owner, sealed));
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------- artifacts

    pub async fn add_artifact(
        &self,
        run_id: &str,
        repo_id: &str,
        name: &str,
        size: u64,
        digest: &str,
    ) -> ApiResult<Artifact> {
        let id = new_id("art");
        let now = now_ms();
        sqlx::query(
            "INSERT INTO artifacts (id, run_id, repo_id, name, size, digest, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (run_id, name) DO UPDATE SET size = excluded.size, digest = excluded.digest",
        )
        .bind(&id)
        .bind(run_id)
        .bind(repo_id)
        .bind(name)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .bind(digest)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(self.artifact(run_id, name).await?.unwrap_or(Artifact {
            id,
            run_id: run_id.to_owned(),
            name: name.to_owned(),
            size: i64::try_from(size).unwrap_or(i64::MAX),
            digest: digest.to_owned(),
            created_at: rfc3339(now),
        }))
    }

    pub async fn artifacts(&self, run_id: &str) -> ApiResult<Vec<Artifact>> {
        let rows = sqlx::query(
            "SELECT id, run_id, name, size, digest, created_at FROM artifacts WHERE run_id = $1 ORDER BY name",
        )
        .bind(run_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(artifact_row)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    pub async fn artifact(&self, run_id: &str, name: &str) -> ApiResult<Option<Artifact>> {
        let row = sqlx::query(
            "SELECT id, run_id, name, size, digest, created_at FROM artifacts WHERE run_id = $1 AND name = $2",
        )
        .bind(run_id)
        .bind(name)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(artifact_row).transpose()?)
    }

    // --------------------------------------------------------- runner tokens

    pub async fn create_runner_token(&self, t: &NewRunnerToken<'_>) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO runner_tokens (hash, owner_id, labels, ephemeral, uses_left, expires_at, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(t.hash)
        .bind(t.owner_id)
        .bind(serde_json::to_string(t.labels)?)
        .bind(i64::from(t.ephemeral))
        .bind(t.uses)
        .bind(t.expires_at)
        .bind(t.by)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Uses one registration of a token, if it is still good.
    pub async fn take_runner_token(&self, hash: &str) -> ApiResult<Option<RunnerGrant>> {
        let taken = sqlx::query(
            "UPDATE runner_tokens SET uses_left = uses_left - 1
             WHERE hash = $1 AND uses_left > 0 AND expires_at > $2",
        )
        .bind(hash)
        .bind(now_ms())
        .execute(self.pool())
        .await?
        .rows_affected();
        if taken == 0 {
            return Ok(None);
        }
        let row =
            sqlx::query("SELECT owner_id, labels, ephemeral FROM runner_tokens WHERE hash = $1")
                .bind(hash)
                .fetch_one(self.pool())
                .await?;
        Ok(Some(RunnerGrant {
            owner_id: row.try_get("owner_id")?,
            labels: json_list(&row, "labels")?,
            ephemeral: row.try_get::<i64, _>("ephemeral")? != 0,
        }))
    }

    pub async fn mark_runner(
        &self,
        runner_id: &str,
        ephemeral: bool,
        machine: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query("UPDATE runners SET ephemeral = $1, machine = $2 WHERE id = $3")
            .bind(i64::from(ephemeral))
            .bind(machine)
            .bind(runner_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// `(runner id, run id)` of the runs an owner's runners hold now.
    pub async fn busy_runners(&self, owner_id: &str) -> ApiResult<Vec<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT c.runner_id, c.id FROM check_runs c JOIN runners r ON r.id = c.runner_id
             WHERE r.owner_id = $1 AND c.state = 'running'",
        )
        .bind(owner_id)
        .fetch_all(self.pool())
        .await?)
    }

    // ---------------------------------------------------------------- reuse

    /// A successful run of the same check with the same inputs, to reuse.
    pub async fn reusable_run(
        &self,
        repo_id: &str,
        name: &str,
        input_key: &str,
    ) -> ApiResult<Option<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT id, commit_id FROM check_runs
             WHERE repo_id = $1 AND name = $2 AND input_key = $3 AND state = 'success'
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(repo_id)
        .bind(name)
        .bind(input_key)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Stores what a run waits for and the key of its inputs.
    pub async fn set_run_pipeline(
        &self,
        run_id: &str,
        needs: &[String],
        input_key: Option<&str>,
        reused_from: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query(
            "UPDATE check_runs SET needs = $1, input_key = $2, reused_from = $3 WHERE id = $4",
        )
        .bind((!needs.is_empty()).then(|| serde_json::to_string(needs).unwrap_or_default()))
        .bind(input_key)
        .bind(reused_from)
        .bind(run_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The checks a run waits for (base names; a name covers its matrix).
    pub async fn run_needs(&self, run_id: &str) -> ApiResult<Vec<String>> {
        let needs: Option<String> =
            sqlx::query_scalar("SELECT needs FROM check_runs WHERE id = $1")
                .bind(run_id)
                .fetch_optional(self.pool())
                .await?
                .flatten();
        Ok(needs
            .and_then(|n| serde_json::from_str(&n).ok())
            .unwrap_or_default())
    }

    /// Records the outcome of a run decided without a runner (reused, or
    /// nothing to do), right after creating it.
    pub async fn close_run(
        &self,
        run_id: &str,
        state: &str,
        exit_code: Option<i64>,
        summary: &str,
    ) -> ApiResult<()> {
        let now = now_ms();
        sqlx::query(
            "UPDATE check_runs SET state = $1, exit_code = $2, summary = $3, started_at = $4, finished_at = $4
             WHERE id = $5",
        )
        .bind(state)
        .bind(exit_code)
        .bind(summary)
        .bind(now)
        .bind(run_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Puts a failed run back in the queue (a retry).
    pub async fn requeue_run(&self, run_id: &str) -> ApiResult<bool> {
        Ok(sqlx::query(
            "UPDATE check_runs SET state = 'queued', runner_id = NULL, lease_until = NULL, finished_at = NULL,
                                   exit_code = NULL
             WHERE id = $1 AND state IN ('failure', 'error')",
        )
        .bind(run_id)
        .execute(self.pool())
        .await?
        .rows_affected()
            == 1)
    }

    // -------------------------------------------------------------- history

    /// Finished runs of a repository's checks, newest first.
    pub async fn run_history(&self, repo_id: &str, limit: i64) -> ApiResult<Vec<RunStat>> {
        let rows = sqlx::query(
            "SELECT name, state, labels, created_at, started_at, finished_at, attempt, reused_from
             FROM check_runs WHERE repo_id = $1 AND finished_at IS NOT NULL
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(repo_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(RunStat {
                    name: r.try_get("name")?,
                    state: r.try_get("state")?,
                    labels: json_list(r, "labels")?,
                    queued_ms: r.try_get("created_at")?,
                    started_ms: r.try_get("started_at")?,
                    finished_ms: r.try_get("finished_at")?,
                    attempt: r.try_get("attempt")?,
                    reused: r.try_get::<Option<String>, _>("reused_from")?.is_some(),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Runs waiting for a runner in an owner's repositories.
    pub async fn queued_runs(&self, owner_id: &str) -> ApiResult<Vec<QueuedRun>> {
        let rows = sqlx::query(
            "SELECT c.id, r.owner_id, r.full_name AS repo_name, c.name, c.labels, c.created_at, c.spec, c.needs
             FROM check_runs c JOIN repos r ON r.id = c.repo_id
             WHERE r.owner_id = $1 AND c.state = 'queued'
             ORDER BY c.created_at LIMIT 500",
        )
        .bind(owner_id)
        .fetch_all(self.pool())
        .await?;
        let now = now_ms();
        rows.iter()
            .map(|r| {
                let spec: String = r.try_get("spec")?;
                let spec: serde_json::Value = serde_json::from_str(&spec).unwrap_or_default();
                let queued: i64 = r.try_get("created_at")?;
                Ok(QueuedRun {
                    id: r.try_get("id")?,
                    repo: r.try_get("repo_name")?,
                    name: r.try_get("name")?,
                    labels: json_list(r, "labels")?,
                    queued_at: rfc3339(queued),
                    waiting_ms: now - queued,
                    image: spec["image"].as_str().map(str::to_owned),
                    blocked: r.try_get::<Option<String>, _>("needs")?.is_some(),
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// Queued runs of a commit that wait for `name` (or its matrix).
    pub async fn dependents(
        &self,
        repo_id: &str,
        commit: &str,
    ) -> ApiResult<Vec<(CheckRun, Vec<String>)>> {
        let runs = self.runs_for_commit(repo_id, commit).await?;
        let mut out = Vec::new();
        for run in runs.into_iter().filter(|r| r.state == "queued") {
            let needs = self.run_needs(&run.id).await?;
            if !needs.is_empty() {
                out.push((run, needs));
            }
        }
        Ok(out)
    }
}

fn artifact_row(r: &sqlx::any::AnyRow) -> Result<Artifact, sqlx::Error> {
    Ok(Artifact {
        id: r.try_get("id")?,
        run_id: r.try_get("run_id")?,
        name: r.try_get("name")?,
        size: r.try_get("size")?,
        digest: r.try_get("digest")?,
        created_at: rfc3339(r.try_get("created_at")?),
    })
}

/// Whether a run named `run` is one of `need` (itself, or one of its
/// matrix: `test (linux)` belongs to `test`).
#[must_use]
pub fn belongs_to(run: &str, need: &str) -> bool {
    run == need
        || run
            .strip_prefix(need)
            .is_some_and(|rest| rest.starts_with(" ("))
}

/// Where a run's dependencies stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Waiting,
    /// A dependency failed, was skipped or cancelled.
    Blocked,
}

/// Decides from the runs of the same commit.
#[must_use]
pub fn readiness(needs: &[String], runs: &[CheckRun]) -> Readiness {
    let mut waiting = false;
    for need in needs {
        let matching: Vec<&CheckRun> = runs.iter().filter(|r| belongs_to(&r.name, need)).collect();
        if matching.is_empty() {
            return Readiness::Blocked;
        }
        for r in matching {
            match r.state.as_str() {
                "success" => {}
                "queued" | "running" | "waiting" => waiting = true,
                _ => return Readiness::Blocked,
            }
        }
    }
    if waiting {
        Readiness::Waiting
    } else {
        Readiness::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_runs_belong_to_their_check() {
        assert!(belongs_to("test", "test"));
        assert!(belongs_to("test (linux, stable)", "test"));
        assert!(!belongs_to("tests", "test"));
        assert!(!belongs_to("lint", "test"));
    }
}
