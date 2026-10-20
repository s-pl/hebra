//! Checks: what `.hb/checks.toml` asks for, when to run it, and the
//! bookkeeping around runs (results, leases, credits).
//!
//! ```toml
//! [[check]]
//! name = "test"
//! run = "cargo test --workspace"
//! on = ["review", "push:main"]     # reviews, and pushes to bookmarks matching a pattern
//! timeout = "20m"
//! labels = ["linux"]               # runners must have every label
//!
//! [[check]]
//! name = "contract"
//! kind = "agent"                   # agent verification (Cloud)
//! contract = ".hb/contract.md"
//! on = ["review"]
//! max_credits = 300
//! ```

use std::sync::Arc;

use hebra_core::ObjectId;
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{BLOB, COMMIT};
use serde::Deserialize;
use serde_json::json;

use crate::api::blocking;
use crate::config::Edition;
use crate::db::{CheckRun, NewCheckRun, Repo, Review};
use crate::error::{ApiError, ApiResult};
use crate::names::glob_match;
use crate::server::Server;

pub const CHECKS_FILE: &str = ".hb/checks.toml";
pub const POLICY_FILE: &str = ".hb/policy.toml";
/// How long a runner holds a run without renewing it.
pub const LEASE_MS: i64 = 60_000;
/// Largest log kept per run.
pub const MAX_LOG_BYTES: i64 = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 20 * 60;
const MAX_TIMEOUT_SECS: u64 = 6 * 3600;
const DEFAULT_RUN_CREDITS: i64 = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSpec {
    pub name: String,
    #[serde(default = "command")]
    pub kind: String,
    pub run: Option<String>,
    pub contract: Option<String>,
    #[serde(default)]
    pub on: Vec<String>,
    pub timeout: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub required: bool,
    pub max_credits: Option<i64>,
}

fn command() -> String {
    "command".to_owned()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksFile {
    #[serde(default)]
    check: Vec<CheckSpec>,
}

/// Rules for one bookmark in `.hb/policy.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookmarkPolicy {
    /// `direct` (pushes allowed), `review` (only through reviews), `queue`.
    #[serde(default = "direct")]
    pub merge: String,
    #[serde(default)]
    pub required_approvals: u32,
    #[serde(default)]
    pub required_checks: Vec<String>,
}

fn direct() -> String {
    "direct".to_owned()
}

/// No policy: pushes allowed, nothing required.
impl Default for BookmarkPolicy {
    fn default() -> Self {
        Self {
            merge: direct(),
            required_approvals: 0,
            required_checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathPolicy {
    #[serde(rename = "match")]
    pub pattern: String,
    /// `deny`: agents may not change these files.
    pub agents: Option<String>,
    pub required_approvals: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub bookmarks: std::collections::BTreeMap<String, BookmarkPolicy>,
    #[serde(default)]
    pub paths: Vec<PathPolicy>,
}

impl Policy {
    /// The rules for `bookmark` (exact name first, then `*` patterns).
    #[must_use]
    pub fn for_bookmark(&self, bookmark: &str) -> BookmarkPolicy {
        self.bookmarks
            .get(bookmark)
            .or_else(|| {
                self.bookmarks
                    .iter()
                    .find(|(pattern, _)| glob_match(pattern, bookmark))
                    .map(|(_, p)| p)
            })
            .cloned()
            .unwrap_or_default()
    }
}

/// Why checks are queued.
enum Trigger<'a> {
    /// A bookmark moved (`on = ["push:<pattern>"]`).
    Push(&'a str),
    /// A review opened or its source moved (`on = ["review"]`).
    Review(&'a Review),
    /// These checks, whatever `on` says (merge queue).
    Named { names: &'a [String], cause: &'a str },
}

/// `20m`, `90s`, `2h`, or plain seconds.
fn parse_timeout(text: Option<&str>) -> ApiResult<u64> {
    let Some(text) = text.map(str::trim) else {
        return Ok(DEFAULT_TIMEOUT_SECS);
    };
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (n, unit) = text.split_at(digits);
    let n: u64 = n
        .parse()
        .map_err(|_| ApiError::bad(format!("invalid timeout `{text}`")))?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => {
            return Err(ApiError::bad(format!(
                "invalid timeout `{text}` (use s, m or h)"
            )));
        }
    };
    Ok(secs.clamp(1, MAX_TIMEOUT_SECS))
}

/// A text file of a commit, if present.
fn read_file(
    store: &hebra_core::store::Store,
    commit: ObjectId,
    path: &str,
) -> ApiResult<Option<String>> {
    let c: Commit = store.read_json(COMMIT, &commit)?;
    let tree: Tree = store.read_tree(&c.tree)?;
    let Some(entry) = tree.files.get(path).filter(|e| !e.symlink) else {
        return Ok(None);
    };
    let bytes = store.read(BLOB, &entry.blob)?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

impl Server {
    async fn file_at(
        &self,
        repo: &Repo,
        commit: ObjectId,
        path: &'static str,
    ) -> ApiResult<Option<String>> {
        let store = self.store(repo);
        blocking(move || read_file(&store, commit, path)).await
    }

    /// The checks declared at a commit (none if the file is absent).
    pub(crate) async fn check_specs(
        &self,
        repo: &Repo,
        commit: ObjectId,
    ) -> ApiResult<Vec<CheckSpec>> {
        let Some(text) = self.file_at(repo, commit, CHECKS_FILE).await? else {
            return Ok(Vec::new());
        };
        let file: ChecksFile =
            toml::from_str(&text).map_err(|e| ApiError::bad(format!("{CHECKS_FILE}: {e}")))?;
        Ok(file.check)
    }

    /// The repository policy as of a commit (usually the target's head).
    pub(crate) async fn policy(&self, repo: &Repo, commit: Option<ObjectId>) -> ApiResult<Policy> {
        let Some(commit) = commit else {
            return Ok(Policy::default());
        };
        match self.file_at(repo, commit, POLICY_FILE).await? {
            Some(text) => {
                toml::from_str(&text).map_err(|e| ApiError::bad(format!("{POLICY_FILE}: {e}")))
            }
            None => Ok(Policy::default()),
        }
    }

    /// Queues the checks that apply to `commit`, because of a push to
    /// `bookmark` or a review. Runs from contributions wait for a
    /// maintainer. Invalid check files produce one failed `checks.toml` run
    /// so the problem is visible where people look.
    pub(crate) async fn trigger_checks(
        &self,
        repo: &Repo,
        commit: ObjectId,
        bookmark: &str,
        review: Option<&Review>,
        needs_approval: bool,
    ) -> ApiResult<Vec<String>> {
        let trigger = match review {
            Some(r) => Trigger::Review(r),
            None => Trigger::Push(bookmark),
        };
        self.queue_checks(repo, commit, &trigger, needs_approval)
            .await
    }

    /// Queues the named checks on `commit` whatever their `on` says (the
    /// merge queue runs a target's required checks on each batch).
    pub(crate) async fn trigger_named_checks(
        &self,
        repo: &Repo,
        commit: ObjectId,
        names: &[String],
        cause: &str,
    ) -> ApiResult<Vec<String>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        self.queue_checks(repo, commit, &Trigger::Named { names, cause }, false)
            .await
    }

    async fn queue_checks(
        &self,
        repo: &Repo,
        commit: ObjectId,
        trigger: &Trigger<'_>,
        needs_approval: bool,
    ) -> ApiResult<Vec<String>> {
        let review = match trigger {
            Trigger::Review(r) => Some(*r),
            _ => None,
        };
        let specs = match self.check_specs(repo, commit).await {
            Ok(s) => s,
            Err(e) => {
                let id = self
                    .db
                    .create_run(&NewCheckRun {
                        repo_id: &repo.id,
                        commit: &commit.hex(),
                        name: "checks.toml",
                        kind: "command",
                        spec: &json!({}),
                        labels: &[],
                        cause: "config",
                        review_id: review.map(|r| r.id.as_str()),
                        required: false,
                        state: "queued",
                    })
                    .await?;
                self.db
                    .finish_run(&id, None, "error", None, &e.message, 0)
                    .await?;
                return Ok(vec![id]);
            }
        };
        let existing = self.db.runs_for_commit(&repo.id, &commit.hex()).await?;
        // For reviews, what the change is compared against (the target now).
        let base = match review {
            Some(r) => self.bookmark_fresh(repo, &r.target).await?.map(|b| b.hex()),
            None => None,
        };
        let cause = match trigger {
            Trigger::Push(b) => format!("push:{b}"),
            Trigger::Review(r) => format!("review:{}", r.number),
            Trigger::Named { cause, .. } => (*cause).to_owned(),
        };
        let mut created = Vec::new();
        for spec in specs {
            let applies = match trigger {
                Trigger::Named { names, .. } => names.contains(&spec.name),
                Trigger::Review(_) => spec.on.iter().any(|on| on == "review"),
                Trigger::Push(bookmark) => spec
                    .on
                    .iter()
                    .filter_map(|on| on.strip_prefix("push:"))
                    .any(|pattern| glob_match(pattern, bookmark)),
            };
            if !applies
                || existing
                    .iter()
                    .any(|r| r.name == spec.name && r.state != "cancelled")
            {
                continue;
            }
            let (kind, mut detail) = match spec.kind.as_str() {
                "command" => {
                    let run = spec.run.clone().ok_or_else(|| {
                        ApiError::bad(format!("check `{}` needs `run`", spec.name))
                    })?;
                    (
                        "command",
                        json!({ "run": run, "timeout_secs": parse_timeout(spec.timeout.as_deref())? }),
                    )
                }
                "agent" => (
                    "agent",
                    json!({
                        "contract": spec.contract.clone().unwrap_or_else(|| ".hb/contract.md".to_owned()),
                        "timeout_secs": parse_timeout(spec.timeout.as_deref())?,
                        "max_credits": spec.max_credits.unwrap_or(DEFAULT_RUN_CREDITS),
                    }),
                ),
                other => {
                    return Err(ApiError::bad(format!(
                        "check `{}`: unknown kind `{other}`",
                        spec.name
                    )));
                }
            };
            if let Some(base) = &base {
                detail["base"] = json!(base);
            }
            let state = if kind == "agent" && self.edition != Edition::Cloud {
                "skipped"
            } else if needs_approval {
                "waiting"
            } else {
                "queued"
            };
            let id = self
                .db
                .create_run(&NewCheckRun {
                    repo_id: &repo.id,
                    commit: &commit.hex(),
                    name: &spec.name,
                    kind,
                    spec: &detail,
                    labels: &spec.labels,
                    cause: &cause,
                    review_id: review.map(|r| r.id.as_str()),
                    required: spec.required,
                    state,
                })
                .await?;
            if state == "skipped" {
                self.db
                    .finish_run(
                        &id,
                        None,
                        "skipped",
                        None,
                        "agent verification is a Hebra Cloud feature",
                        0,
                    )
                    .await?;
            }
            created.push(id);
        }
        if !created.is_empty() {
            self.notify(
                repo,
                "check",
                json!({ "commit": commit.hex(), "queued": created.len() }),
            );
        }
        Ok(created)
    }

    /// Runs whose runner vanished go back to the queue (or fail after too
    /// many attempts); tells live clients.
    pub(crate) async fn requeue_expired_checks(&self) -> ApiResult<()> {
        for run in self.db.expire_runs().await? {
            if let Some(repo) = self.db.repo_by_id(&run.repo_id).await? {
                self.notify(
                    &repo,
                    "check",
                    json!({ "id": run.id, "name": run.name, "commit": run.commit }),
                );
            }
        }
        Ok(())
    }

    /// The required checks for merging into a bookmark, and how they stand
    /// on `commit`: (missing or not passing, all runs).
    pub(crate) async fn required_checks(
        &self,
        repo: &Repo,
        policy: &BookmarkPolicy,
        commit: ObjectId,
    ) -> ApiResult<(Vec<String>, Vec<CheckRun>)> {
        let runs = self.db.runs_for_commit(&repo.id, &commit.hex()).await?;
        let mut required: Vec<String> = policy.required_checks.clone();
        required.extend(runs.iter().filter(|r| r.required).map(|r| r.name.clone()));
        required.sort();
        required.dedup();
        let failing = required
            .into_iter()
            .filter(|name| !runs.iter().any(|r| &r.name == name && r.state == "success"))
            .collect();
        Ok((failing, runs))
    }
}

/// For handlers that hold an `Arc<Server>`.
pub(crate) async fn trigger_for_push(
    server: Arc<Server>,
    repo: Arc<Repo>,
    commit: ObjectId,
    bookmark: String,
) {
    if let Err(e) = server
        .trigger_checks(&repo, commit, &bookmark, None, false)
        .await
    {
        tracing::warn!(
            repo = repo.full_name,
            error = e.message,
            "could not queue checks"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts() {
        assert_eq!(parse_timeout(None).unwrap(), DEFAULT_TIMEOUT_SECS);
        assert_eq!(parse_timeout(Some("20m")).unwrap(), 1200);
        assert_eq!(parse_timeout(Some("90")).unwrap(), 90);
        assert_eq!(parse_timeout(Some("2h")).unwrap(), 7200);
        assert_eq!(parse_timeout(Some("100h")).unwrap(), MAX_TIMEOUT_SECS);
        assert!(parse_timeout(Some("soon")).is_err());
    }

    #[test]
    fn policy_lookup() {
        let p: Policy = toml::from_str(
            r#"
            [bookmarks.main]
            merge = "review"
            required_approvals = 1
            required_checks = ["test"]
            [bookmarks."release-*"]
            merge = "queue"
            [[paths]]
            match = "infra/**"
            agents = "deny"
            "#,
        )
        .unwrap();
        assert_eq!(p.for_bookmark("main").required_approvals, 1);
        assert_eq!(p.for_bookmark("release-2").merge, "queue");
        assert_eq!(p.for_bookmark("dev").merge, "direct");
        assert_eq!(Policy::default().for_bookmark("main").merge, "direct");
        assert_eq!(p.paths[0].agents.as_deref(), Some("deny"));
    }

    #[test]
    fn checks_file() {
        let f: ChecksFile = toml::from_str(
            r#"
            [[check]]
            name = "test"
            run = "cargo test"
            on = ["review", "push:main"]
            [[check]]
            name = "contract"
            kind = "agent"
            on = ["review"]
            "#,
        )
        .unwrap();
        assert_eq!(f.check.len(), 2);
        assert_eq!(f.check[0].kind, "command");
        assert!(toml::from_str::<ChecksFile>("[[check]]\nname='x'\nunknown=1").is_err());
    }
}
