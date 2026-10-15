//! `hb runner`: run checks on your own machines.
//!
//! `hb runner register` creates a runner for a person or organization and
//! keeps its mandate in `<config dir>/hebra/runners/<name>.json`.
//! `hb runner start` then loops: claim a run, check the commit out into a
//! fresh directory, run it (a command, or the verifier agent for agent
//! checks), stream the log, renew the lease, report the result.
//!
//! Runs get a clean environment: no credentials, only `PATH`, temp and
//! home directories, and `CI=true` plus `HEBRA_*` details of the run.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{BLOB, COMMIT, Store, TREE};
use hebra_core::{ObjectId, transport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::Client;
use crate::commands::Output;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Largest output a tool call returns to the verifier model.
const TOOL_OUTPUT_LIMIT: usize = 30 * 1024;
/// Most model turns in one verification.
const MAX_TURNS: usize = 60;

#[derive(Debug, Serialize, Deserialize)]
struct RunnerConfig {
    server: String,
    name: String,
    mandate: String,
    labels: Vec<String>,
}

fn runners_dir() -> Result<PathBuf> {
    let dir = std::env::var_os("HB_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            #[cfg(windows)]
            return std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("hebra"));
            #[cfg(not(windows))]
            return std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                .map(|d| d.join("hebra"));
        })
        .ok_or_else(|| anyhow!("cannot find a configuration directory; set HB_CONFIG_DIR"))?;
    Ok(dir.join("runners"))
}

pub fn register(server: &str, owner: &str, name: &str, labels: &[String]) -> Result<Output> {
    let server = server.trim_end_matches('/');
    let token = crate::account::credential(server)
        .ok_or_else(|| anyhow!("run `hb login {server}` first"))?;
    let reply = Client::new(server, Some(token)).post(
        &format!("/accounts/{owner}/runners"),
        &json!({ "name": name, "labels": labels }),
    )?;
    let mandate = reply["mandate"]
        .as_str()
        .ok_or_else(|| anyhow!("no mandate in reply"))?;
    let config = RunnerConfig {
        server: server.to_owned(),
        name: name.to_owned(),
        mandate: mandate.to_owned(),
        labels: labels.to_vec(),
    };
    let dir = runners_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.json"));
    write_private(&path, &serde_json::to_vec_pretty(&config)?)?;
    Ok(Output::new(
        json!({ "runner": name, "owner": owner, "labels": labels, "config": path }),
        format!(
            "Registered runner {name} for {owner} (labels: {}).\nStart it with `hb runner start {name}`.\n",
            if labels.is_empty() {
                "none".to_owned()
            } else {
                labels.join(", ")
            }
        ),
    ))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(bytes)?;
    Ok(())
}

fn load_config(name: &str) -> Result<RunnerConfig> {
    let path = runners_dir()?.join(format!("{name}.json"));
    let bytes = std::fs::read(&path).with_context(|| {
        format!("no runner `{name}` here; register it with `hb runner register`")
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// `hb runner start`: claims and runs checks until stopped (or one run
/// with `once`).
pub fn start(name: &str, once: bool) -> Result<Output> {
    let config = load_config(name)?;
    let client = Client::new(&config.server, Some(config.mandate.clone()));
    eprintln!("runner {name}: waiting for checks on {}", config.server);
    let mut done = 0u64;
    loop {
        let claim = match client.post("/runner/claim", &json!({ "version": VERSION })) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("runner {name}: {e:#}; retrying");
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
        };
        if claim["run"].is_null() {
            if once {
                return Ok(Output::new(
                    json!({ "ran": done }),
                    format!("No checks waiting. Ran {done}.\n"),
                ));
            }
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        let run = &claim["run"];
        let repo = claim["repo"].as_str().unwrap_or_default().to_owned();
        let id = run["id"].as_str().unwrap_or_default().to_owned();
        eprintln!(
            "runner {name}: {} on {repo}@{}",
            run["name"].as_str().unwrap_or("?"),
            run["commit"]
                .as_str()
                .map_or("?", |c| c.get(..12).unwrap_or(c))
        );
        let (state, exit_code, summary) = match execute(&client, &repo, run) {
            Ok(result) => result,
            Err(e) => ("error".to_owned(), None, format!("{e:#}")),
        };
        let _ = client.post(
            &format!("/runner/runs/{id}/complete"),
            &json!({ "state": state, "exit_code": exit_code, "summary": summary }),
        );
        eprintln!("runner {name}: {state}");
        done += 1;
        if once {
            return Ok(Output::new(
                json!({ "ran": done, "state": state }),
                format!("Ran 1 check: {state}.\n"),
            ));
        }
    }
}

/// Paths in trees are relative and never leave the checkout.
fn safe_path(root: &Path, path: &str) -> Result<PathBuf> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || path.contains(':')
    {
        bail!("refusing unsafe path `{path}` in the commit");
    }
    Ok(root.join(path))
}

/// Downloads `commit` and writes its files into `dir`. Returns the store
/// (for diffs) and the commit.
fn checkout(
    client: &Client,
    repo: &str,
    commit: &str,
    base: Option<&str>,
    dir: &Path,
) -> Result<(Store, Commit)> {
    let mut want = vec![commit.to_owned()];
    want.extend(base.map(str::to_owned));
    let pack = client.post_bytes(
        &format!("/repos/{repo}/pack"),
        &json!({ "want": want, "have": [] }),
    )?;
    let objects = transport::decode(&pack)?;
    let store = Store::new(dir.join(".hebra-objects"));
    transport::store_all(&store, &objects)?;
    let id = ObjectId::parse(commit)?;
    let c: Commit = store.read_json(COMMIT, &id)?;
    let tree: Tree = store.read_json(TREE, &c.tree)?;
    let work = dir.join("work");
    std::fs::create_dir_all(&work)?;
    for (path, entry) in &tree.files {
        let target = safe_path(&work, path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, store.read(BLOB, &entry.blob)?)?;
        #[cfg(unix)]
        if entry.executable {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok((store, c))
}

/// Sends log text in batches and renews the lease; stops the run if the
/// server says it is no longer ours.
struct Reporter {
    client: Arc<Client>,
    id: String,
    buffer: Arc<Mutex<String>>,
    cancelled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Reporter {
    fn start(client: Arc<Client>, id: &str) -> (Self, std::thread::JoinHandle<()>) {
        let reporter = Self {
            client,
            id: id.to_owned(),
            buffer: Arc::new(Mutex::new(String::new())),
            cancelled: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (client, id, buffer, cancelled, stop) = (
            Arc::clone(&reporter.client),
            reporter.id.clone(),
            Arc::clone(&reporter.buffer),
            Arc::clone(&reporter.cancelled),
            Arc::clone(&reporter.stop),
        );
        let handle = std::thread::spawn(move || {
            let mut last_beat = Instant::now() - Duration::from_secs(60);
            loop {
                let stopping = stop.load(Ordering::Relaxed);
                let text = std::mem::take(
                    &mut *buffer
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                if !text.is_empty() {
                    for chunk in chunks(&text, 200 * 1024) {
                        let _ = client
                            .post(&format!("/runner/runs/{id}/log"), &json!({ "text": chunk }));
                    }
                }
                if stopping {
                    break;
                }
                if last_beat.elapsed() >= Duration::from_secs(15) {
                    last_beat = Instant::now();
                    if let Ok(reply) =
                        client.post(&format!("/runner/runs/{id}/heartbeat"), &json!({}))
                        && reply["continue"] == false
                    {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        (reporter, handle)
    }

    fn log(&self, text: &str) {
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_str(text);
    }
}

fn chunks(text: &str, size: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if current.len() + c.len_utf8() > size {
            out.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A command under the platform shell, with a clean environment.
fn shell(command: &str, dir: &Path, run: &Value, repo: &str) -> Command {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(dir).env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "TEMP",
        "TMP",
        "TMPDIR",
        "SYSTEMROOT",
        "COMSPEC",
        "LANG",
    ] {
        if let Some(v) = std::env::var_os(key) {
            cmd.env(key, v);
        }
    }
    cmd.env("CI", "true")
        .env("HEBRA_REPO", repo)
        .env("HEBRA_COMMIT", run["commit"].as_str().unwrap_or_default())
        .env("HEBRA_CHECK", run["name"].as_str().unwrap_or_default())
        .env("HEBRA_RUN", run["id"].as_str().unwrap_or_default());
    cmd
}

/// Runs a child, copying its output into `sink`, until it ends, times out,
/// or `cancelled` is set. Returns the exit code (`None` if killed).
fn supervise(
    mut child: Child,
    timeout: Duration,
    cancelled: &AtomicBool,
    sink: &dyn Fn(&str),
) -> Result<(Option<i32>, String)> {
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut readers = Vec::new();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let output = Arc::clone(&output);
        readers.push(std::thread::spawn(move || {
            let mut stream = stream;
            let mut buf = [0u8; 8192];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(&buf[..n]);
            }
        }));
    }
    let started = Instant::now();
    let mut sent = 0;
    let mut reason = String::new();
    let code = loop {
        {
            let out = output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if out.len() > sent {
                sink(&String::from_utf8_lossy(&out[sent..]));
                sent = out.len();
            }
        }
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            reason = format!("timed out after {}s", timeout.as_secs());
            break None;
        }
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            reason = "cancelled".to_owned();
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    for r in readers {
        let _ = r.join();
    }
    let out = output
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if out.len() > sent {
        sink(&String::from_utf8_lossy(&out[sent..]));
    }
    Ok((code, reason))
}

/// Executes one claimed run: (state, exit code, summary).
fn execute(client: &Client, repo: &str, run: &Value) -> Result<(String, Option<i64>, String)> {
    let id = run["id"].as_str().unwrap_or_default();
    let commit = run["commit"]
        .as_str()
        .ok_or_else(|| anyhow!("run without commit"))?;
    let spec = &run["spec"];
    let base = spec["base"].as_str();
    let dir = tempfile::tempdir()?;
    let client = Arc::new(Client::new(&client.base, client_token(client)));
    let (reporter, handle) = Reporter::start(Arc::clone(&client), id);
    let result = (|| {
        reporter.log(&format!(
            "$ hb checkout {}\n",
            commit.get(..12).unwrap_or(commit)
        ));
        let (store, c) = checkout(&client, repo, commit, base, dir.path())?;
        let work = dir.path().join("work");
        match run["kind"].as_str() {
            Some("agent") => crate::verify::run(
                &client,
                &reporter_sink(&reporter),
                id,
                &work,
                &store,
                &c,
                base,
                spec,
            ),
            _ => {
                let command = spec["run"]
                    .as_str()
                    .ok_or_else(|| anyhow!("check without command"))?;
                let timeout = Duration::from_secs(spec["timeout_secs"].as_u64().unwrap_or(1200));
                reporter.log(&format!("$ {command}\n"));
                let child = shell(command, &work, run, repo)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .with_context(|| format!("starting `{command}`"))?;
                let (code, reason) =
                    supervise(child, timeout, &reporter.cancelled, &|t| reporter.log(t))?;
                Ok(match code {
                    Some(0) => ("success".to_owned(), Some(0), "passed".to_owned()),
                    Some(c) => (
                        "failure".to_owned(),
                        Some(i64::from(c)),
                        format!("exited with {c}"),
                    ),
                    None => ("failure".to_owned(), None, reason),
                })
            }
        }
    })();
    if let Err(e) = &result {
        reporter.log(&format!("\nerror: {e:#}\n"));
    }
    reporter.stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    result
}

fn client_token(client: &Client) -> Option<String> {
    client.token().map(str::to_owned)
}

/// What the verifier uses to write to the run log.
pub type Sink<'a> = dyn Fn(&str) + 'a;

fn reporter_sink(reporter: &Reporter) -> impl Fn(&str) + '_ {
    move |t: &str| reporter.log(t)
}

/// Runs one shell command for the verifier, with a time limit; returns
/// exit code and output (truncated).
pub fn tool_run(command: &str, work: &Path, timeout: Duration) -> Result<String> {
    let child = shell(command, work, &Value::Null, "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting `{command}`"))?;
    let collected = Mutex::new(String::new());
    let (code, reason) = supervise(child, timeout, &AtomicBool::new(false), &|t| {
        collected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_str(t);
    })?;
    let out = collected
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(format!(
        "exit code: {}{}\n{}",
        code.map_or_else(|| "none".to_owned(), |c| c.to_string()),
        if reason.is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        },
        truncate(&out)
    ))
}

pub fn truncate(text: &str) -> String {
    if text.len() <= TOOL_OUTPUT_LIMIT {
        return text.to_owned();
    }
    let head: String = text.chars().take(TOOL_OUTPUT_LIMIT / 3).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(TOOL_OUTPUT_LIMIT * 2 / 3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(
        "{head}\n… ({} bytes omitted) …\n{tail}",
        text.len() - head.len() - tail.len()
    )
}

pub const fn max_turns() -> usize {
    MAX_TURNS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths_are_refused() {
        let root = Path::new("/tmp/x");
        assert!(safe_path(root, "src/main.rs").is_ok());
        for bad in [
            "../etc/passwd",
            "/etc/passwd",
            "a/../../b",
            "a//b",
            "C:/x",
            "a\\b",
            "",
        ] {
            assert!(safe_path(root, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn long_output_is_truncated_in_the_middle() {
        let long = "x".repeat(TOOL_OUTPUT_LIMIT * 2);
        let t = truncate(&long);
        assert!(t.len() < long.len());
        assert!(t.contains("omitted"));
    }
}
