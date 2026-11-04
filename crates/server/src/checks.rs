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
//! name = "build"
//! needs = ["test"]                 # starts once `test` passed (all of its matrix)
//! image = "rust:1.95"              # in a container, on runners with Docker
//! matrix = { target = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] }
//! env = { CARGO_TERM_COLOR = "always" }
//! secrets = ["REGISTRY_TOKEN"]     # from repository or owner secrets
//! paths = ["crates/**", "Cargo.lock"]   # only when these changed
//! inputs = ["crates/**", "Cargo.*"]     # same inputs as a passing run: reused
//! artifacts = ["target/release/hb*"]
//! cache = [{ key = "cargo-${{ matrix.target }}", paths = ["target"] }]
//! retries = 1
//!
//! [[check]]
//! name = "deploy"
//! needs = ["build"]
//! environment = "production"       # its rules, secrets and runners apply
//! run = "./deploy.sh"
//! steps = [
//!   { name = "build", run = "cargo build --release --target ${{ matrix.target }}" },
//!   { name = "notes", run = "echo built", if = "always" },
//! ]
//! [[check.services]]
//! name = "db"
//! image = "postgres:17"
//! env = { POSTGRES_PASSWORD = "test" }
//!
//! [[check]]
//! name = "contract"
//! kind = "agent"                   # agent verification (Cloud)
//! contract = ".hb/contract.md"
//! on = ["review"]
//! max_credits = 300
//! ```
//!
//! Reuse is Heddle's first rule: a check whose inputs (the files it reads,
//! by content, and its own definition) match a run that passed is not run
//! again. Hebra's trees are Merkle trees, so comparing inputs costs nothing.

use std::collections::BTreeMap;
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
    /// Commands run one after another (instead of, or after, `run`).
    #[serde(default)]
    pub steps: Vec<StepSpec>,
    /// Checks that must pass first (a name covers its whole matrix).
    #[serde(default)]
    pub needs: Vec<String>,
    /// One run per combination; `${{ matrix.<key> }}` in commands, labels,
    /// image and env takes each value.
    #[serde(default)]
    pub matrix: BTreeMap<String, Vec<String>>,
    /// A container image to run in (runners with Docker).
    pub image: Option<String>,
    #[serde(default)]
    pub services: Vec<ServiceSpec>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Names of secrets handed to the run as environment variables.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Files kept after the run (globs relative to the checkout).
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// Directories kept between runs on the same runner, by key.
    #[serde(default)]
    pub cache: Vec<CacheSpec>,
    /// Run only when a file under these changed.
    #[serde(default)]
    pub paths: Vec<String>,
    /// The files the result depends on (all of them when empty).
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Reuse a passing run with the same inputs (on by default).
    #[serde(default = "yes")]
    pub reuse: bool,
    /// Times a failed run is tried again.
    #[serde(default)]
    pub retries: u32,
    /// Deploys to this environment: its rules (who approves, which
    /// bookmarks may deploy, which runners) and secrets apply, results are
    /// never reused, and each run is a deployment in its history.
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepSpec {
    pub name: Option<String>,
    pub run: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// `success` (default: only while everything passed), `always`, `failure`.
    #[serde(rename = "if")]
    pub when: Option<String>,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpec {
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheSpec {
    pub key: String,
    pub paths: Vec<String>,
}

fn command() -> String {
    "command".to_owned()
}

const fn yes() -> bool {
    true
}

/// Most runs one check may expand to.
const MAX_MATRIX: usize = 64;

/// `${{ matrix.key }}` (or `${matrix.key}`) replaced by its value.
fn substitute(text: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = text.to_owned();
    for (k, v) in values {
        for pattern in [
            format!("${{{{ matrix.{k} }}}}"),
            format!("${{{{matrix.{k}}}}}"),
            format!("${{matrix.{k}}}"),
        ] {
            out = out.replace(&pattern, v);
        }
    }
    out
}

/// One run of a check: its name and values for a matrix combination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub name: String,
    pub values: BTreeMap<String, String>,
}

/// Every combination of a check's matrix (just the check without one).
pub fn expand(spec: &CheckSpec) -> ApiResult<Vec<Instance>> {
    let mut combos: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
    for (key, values) in &spec.matrix {
        if values.is_empty() {
            return Err(ApiError::bad(format!(
                "check `{}`: matrix `{key}` has no values",
                spec.name
            )));
        }
        combos = combos
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut c = c.clone();
                    c.insert(key.clone(), v.clone());
                    c
                })
            })
            .collect();
        if combos.len() > MAX_MATRIX {
            return Err(ApiError::bad(format!(
                "check `{}`: the matrix makes more than {MAX_MATRIX} runs",
                spec.name
            )));
        }
    }
    Ok(combos
        .into_iter()
        .map(|values| Instance {
            name: if values.is_empty() {
                spec.name.clone()
            } else {
                let joined: Vec<&str> = values.values().map(String::as_str).collect();
                format!("{} ({})", spec.name, joined.join(", "))
            },
            values,
        })
        .collect())
}

/// Whether a path is under one of `patterns` (`*` globs, or a directory).
#[must_use]
pub fn path_matches(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| {
        glob_match(p, path) || {
            let dir = p.trim_end_matches('/');
            !dir.contains('*') && path.starts_with(&format!("{dir}/"))
        }
    })
}

/// What a runner needs to run one instance of a command check.
fn command_detail(
    spec: &CheckSpec,
    values: &BTreeMap<String, String>,
) -> ApiResult<serde_json::Value> {
    if spec.run.is_none() && spec.steps.is_empty() {
        return Err(ApiError::bad(format!(
            "check `{}` needs `run` or `steps`",
            spec.name
        )));
    }
    let sub = |t: &str| substitute(t, values);
    let env = |m: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        m.iter().map(|(k, v)| (k.clone(), sub(v))).collect()
    };
    let steps: Vec<StepSpec> = spec
        .steps
        .iter()
        .map(|s| StepSpec {
            name: s.name.clone(),
            run: sub(&s.run),
            env: env(&s.env),
            when: s.when.clone(),
            workdir: s.workdir.clone(),
        })
        .collect();
    if let Some(bad) = steps
        .iter()
        .filter_map(|s| s.when.as_deref())
        .find(|w| !["success", "always", "failure"].contains(w))
    {
        return Err(ApiError::bad(format!(
            "check `{}`: step `if` is success, always or failure, not `{bad}`",
            spec.name
        )));
    }
    let mut detail = json!({
        "timeout_secs": parse_timeout(spec.timeout.as_deref())?,
    });
    if let Some(run) = &spec.run {
        detail["run"] = json!(sub(run));
    }
    if !steps.is_empty() {
        detail["steps"] = json!(steps);
    }
    if let Some(image) = &spec.image {
        detail["image"] = json!(sub(image));
    }
    if !spec.services.is_empty() {
        let services: Vec<ServiceSpec> = spec
            .services
            .iter()
            .map(|s| ServiceSpec {
                name: s.name.clone(),
                image: sub(&s.image),
                env: env(&s.env),
            })
            .collect();
        detail["services"] = json!(services);
    }
    if !spec.env.is_empty() {
        detail["env"] = json!(env(&spec.env));
    }
    if !spec.secrets.is_empty() {
        detail["secrets"] = json!(spec.secrets);
    }
    if !spec.artifacts.is_empty() {
        detail["artifacts"] = json!(spec.artifacts);
    }
    if !spec.cache.is_empty() {
        let cache: Vec<CacheSpec> = spec
            .cache
            .iter()
            .map(|c| CacheSpec {
                key: sub(&c.key),
                paths: c.paths.clone(),
            })
            .collect();
        detail["cache"] = json!(cache);
    }
    if !values.is_empty() {
        detail["matrix"] = json!(values);
    }
    if spec.retries > 0 {
        detail["retries"] = json!(spec.retries.min(5));
    }
    Ok(detail)
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
    let Some(entry) = tree.files.get(path).filter(|e| e.is_file()) else {
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
        let mut changed: Option<Vec<String>> = None;
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
            if !applies {
                continue;
            }
            // Nothing changed where this check looks: skipped, not run.
            let mut untouched = false;
            if !spec.paths.is_empty() && !matches!(trigger, Trigger::Named { .. }) {
                if changed.is_none() {
                    changed = Some(self.changed_paths(repo, commit, base.as_deref()).await?);
                }
                untouched = !changed
                    .iter()
                    .flatten()
                    .any(|path| path_matches(&spec.paths, path));
            }
            for instance in expand(&spec)? {
                if existing
                    .iter()
                    .any(|r| r.name == instance.name && r.state != "cancelled")
                {
                    continue;
                }
                let (kind, mut detail) = match spec.kind.as_str() {
                    "command" => ("command", command_detail(&spec, &instance.values)?),
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
                let mut labels: Vec<String> = spec
                    .labels
                    .iter()
                    .map(|l| substitute(l, &instance.values))
                    .collect();
                // Environments: who may deploy, from where, on which runners.
                let from = match trigger {
                    Trigger::Push(b) => Some((*b).to_owned()),
                    Trigger::Review(r) => Some(r.source.clone()),
                    Trigger::Named { .. } => None,
                };
                let mut env_block: Option<String> = None;
                let mut env_approvals = 0;
                if let Some(env_name) = &spec.environment {
                    detail["environment"] = json!(env_name);
                    if let Some(env) = self.db.environment(&repo.id, env_name).await? {
                        env_approvals = env.required_approvals;
                        for l in &env.runner_labels {
                            if !labels.contains(l) {
                                labels.push(l.clone());
                            }
                        }
                        let allowed = env.branches.is_empty()
                            || from
                                .as_deref()
                                .is_some_and(|b| env.branches.iter().any(|p| glob_match(p, b)));
                        if !allowed {
                            env_block = Some(format!(
                                "skipped: {env_name} only deploys from {}",
                                env.branches.join(", ")
                            ));
                        }
                        if let Some(url) = &env.url {
                            detail["environment_url"] = json!(url);
                        }
                    }
                }
                // Reuse: the same definition on the same inputs already passed
                // (never for deployments).
                let input_key = if kind == "command" && spec.reuse && spec.environment.is_none() {
                    Some(
                        self.input_key(repo, commit, &instance.name, &detail, &spec.inputs)
                            .await?,
                    )
                } else {
                    None
                };
                let reused = match &input_key {
                    Some(key) if !untouched => {
                        self.db.reusable_run(&repo.id, &instance.name, key).await?
                    }
                    _ => None,
                };
                if let Some(base) = &base {
                    detail["base"] = json!(base);
                }
                let state = if untouched || env_block.is_some() {
                    "skipped"
                } else if reused.is_some() {
                    "success"
                } else if kind == "agent" && self.edition != Edition::Cloud {
                    "skipped"
                } else if needs_approval || env_approvals > 0 {
                    "waiting"
                } else {
                    "queued"
                };
                let id = self
                    .db
                    .create_run(&NewCheckRun {
                        repo_id: &repo.id,
                        commit: &commit.hex(),
                        name: &instance.name,
                        kind,
                        spec: &detail,
                        labels: &labels,
                        cause: &cause,
                        review_id: review.map(|r| r.id.as_str()),
                        required: spec.required,
                        state,
                    })
                    .await?;
                self.db
                    .set_run_pipeline(
                        &id,
                        &spec.needs,
                        input_key.as_deref(),
                        reused.as_ref().map(|(run, _)| run.as_str()),
                    )
                    .await?;
                if let Some(env_name) = &spec.environment {
                    self.db
                        .record_deployment(&repo.id, env_name, &id, &commit.hex(), from.as_deref())
                        .await?;
                }
                let summary = if let Some(block) = &env_block {
                    Some(block.clone())
                } else if untouched {
                    Some(format!(
                        "skipped: nothing changed under {}",
                        spec.paths.join(", ")
                    ))
                } else if let Some((run, at)) = &reused {
                    Some(format!(
                        "reused: same inputs as run {run} on {}, which passed",
                        at.get(..12).unwrap_or(at)
                    ))
                } else if state == "skipped" {
                    Some("agent verification is a Hebra Cloud feature".to_owned())
                } else {
                    None
                };
                if let Some(summary) = summary {
                    let exit = (state == "success").then_some(0);
                    self.db.close_run(&id, state, exit, &summary).await?;
                }
                created.push(id);
            }
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

    /// Paths a commit changed: against the review's target, or its first
    /// parent.
    async fn changed_paths(
        &self,
        repo: &Repo,
        commit: ObjectId,
        base: Option<&str>,
    ) -> ApiResult<Vec<String>> {
        let store = self.store(repo);
        let base = base.map(ObjectId::parse).transpose()?;
        blocking(move || {
            let c: Commit = store.read_json(COMMIT, &commit)?;
            let from = match base.or_else(|| c.parents.first().copied()) {
                Some(b) => store.read_json::<Commit>(COMMIT, &b).ok().map(|p| p.tree),
                None => None,
            };
            let changes = match from {
                Some(tree) => store.tree_changes(&tree, &c.tree)?,
                None => store
                    .read_tree(&c.tree)?
                    .files
                    .into_iter()
                    .map(|(p, e)| (p, Some(e)))
                    .collect(),
            };
            Ok(changes.into_iter().map(|(p, _)| p).collect())
        })
        .await
    }

    /// What a run's result depends on: its name, its definition, and the
    /// files under `inputs` (by content), or the whole tree.
    async fn input_key(
        &self,
        repo: &Repo,
        commit: ObjectId,
        name: &str,
        detail: &serde_json::Value,
        inputs: &[String],
    ) -> ApiResult<String> {
        let store = self.store(repo);
        let inputs = inputs.to_vec();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"hebra check inputs v1\0");
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update(detail.to_string().as_bytes());
        blocking(move || {
            let c: Commit = store.read_json(COMMIT, &commit)?;
            if inputs.is_empty() {
                // The tree id names every file's content at once.
                hasher.update(b"\0tree\0");
                hasher.update(c.tree.hex().as_bytes());
            } else {
                let tree = store.read_tree(&c.tree)?;
                for (path, entry) in &tree.files {
                    if path_matches(&inputs, path) {
                        hasher.update(path.as_bytes());
                        hasher.update(b"\0");
                        hasher.update(entry.blob.as_bytes());
                        hasher.update(&[u8::from(entry.executable)]);
                    }
                }
            }
            Ok(hasher.finalize().to_hex().to_string())
        })
        .await
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
            .filter(|name| {
                // A required check passes when every run of it (its matrix)
                // passed or had nothing to do.
                let mine: Vec<&CheckRun> = runs
                    .iter()
                    .filter(|r| crate::db::belongs_to(&r.name, name))
                    .collect();
                mine.is_empty()
                    || !mine.iter().all(|r| {
                        r.state == "success"
                            || (r.state == "skipped"
                                && r.summary
                                    .as_deref()
                                    .is_some_and(|s| s.starts_with("skipped: nothing changed")))
                    })
            })
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
    fn matrices_expand_and_substitute() {
        let f: ChecksFile = toml::from_str(
            r#"
            [[check]]
            name = "test"
            image = "rust:${{ matrix.rust }}"
            labels = ["${{ matrix.os }}"]
            matrix = { os = ["linux", "windows"], rust = ["1.95", "beta"] }
            steps = [{ run = "cargo +${{ matrix.rust }} test", if = "always" }]
            "#,
        )
        .unwrap();
        let spec = &f.check[0];
        let runs = expand(spec).unwrap();
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[0].name, "test (linux, 1.95)");
        let detail = command_detail(spec, &runs[3].values).unwrap();
        assert_eq!(detail["image"], "rust:beta");
        assert_eq!(detail["steps"][0]["run"], "cargo +beta test");
        assert!(path_matches(
            &["crates/**".to_owned()],
            "crates/core/src/lib.rs"
        ));
        assert!(path_matches(&["docs".to_owned()], "docs/guide.md"));
        assert!(!path_matches(&["docs".to_owned()], "docsy.md"));
        assert!(command_detail(&f.check[0], &BTreeMap::new()).is_ok());
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
