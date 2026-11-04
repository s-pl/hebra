//! `hb runner`: run checks on your own machines, a VPS, or machines an
//! orchestrator starts and stops.
//!
//! `hb runner register` creates a runner for a person or organization (with
//! the person's sign-in, or with a one-time `--token` so a fresh machine
//! needs no person) and keeps its mandate in
//! `<config dir>/hebra/runners/<name>.json`. `--install` also makes it a
//! service that starts with the machine. `hb runner start` then loops:
//! claim a run, check the commit out into a fresh directory, run it, stream
//! the log, renew the lease, report the result.
//!
//! A run is a command or a list of steps, on the machine or inside a
//! container (`image`, with `services` beside it on a private network).
//! Runs get a clean environment: only `PATH`, temp and home directories,
//! `CI=true`, `HEBRA_*` details of the run, the check's `env`, and the
//! secrets it names, which are masked in the log. Directories listed in
//! `cache` are kept on the runner between runs; files matching `artifacts`
//! are uploaded; artifacts of the checks a run needs are downloaded into
//! `.artifacts/` first. An ephemeral runner takes one run and removes
//! itself.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::model::{Commit, Tree};
use hebra_core::store::{COMMIT, Store};
use hebra_core::{ObjectId, transport, wc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::Client;
use crate::commands::Output;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Largest output a tool call returns to the verifier model.
const TOOL_OUTPUT_LIMIT: usize = 30 * 1024;
/// Most model turns in one verification.
const MAX_TURNS: usize = 60;
/// Secrets shorter than this are not masked (they would mask everything).
const MIN_MASKED: usize = 4;

#[derive(Debug, Serialize, Deserialize)]
struct RunnerConfig {
    server: String,
    name: String,
    mandate: String,
    labels: Vec<String>,
    #[serde(default)]
    ephemeral: bool,
}

fn config_dir() -> Result<PathBuf> {
    std::env::var_os("HB_CONFIG_DIR")
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
        .ok_or_else(|| anyhow!("cannot find a configuration directory; set HB_CONFIG_DIR"))
}

fn runners_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("runners"))
}

/// Where a runner keeps directories between runs.
fn cache_dir(name: &str) -> Result<PathBuf> {
    Ok(std::env::var_os("HB_RUNNER_CACHE")
        .map_or(runners_dir()?.join(format!("{name}-cache")), PathBuf::from))
}

fn save_config(config: &RunnerConfig) -> Result<PathBuf> {
    let dir = runners_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", config.name));
    write_private(&path, &serde_json::to_vec_pretty(config)?)?;
    Ok(path)
}

pub struct Registration<'a> {
    pub server: &'a str,
    pub owner: Option<&'a str>,
    pub name: &'a str,
    pub labels: &'a [String],
    pub token: Option<&'a str>,
    pub machine: Option<&'a str>,
    pub install: bool,
}

pub fn register(r: &Registration<'_>) -> Result<Output> {
    let server = r.server.trim_end_matches('/');
    let reply = if let Some(token) = r.token {
        Client::new(server, None).post(
            "/runner/register",
            &json!({ "token": token, "name": r.name, "labels": r.labels, "machine": r.machine }),
        )?
    } else {
        let owner = r
            .owner
            .ok_or_else(|| anyhow!("say whose runner this is with --owner, or use --token"))?;
        let token = crate::account::credential(server)
            .ok_or_else(|| anyhow!("run `hb login {server}` first, or use --token"))?;
        let mut reply = Client::new(server, Some(token)).post(
            &format!("/accounts/{owner}/runners"),
            &json!({ "name": r.name, "labels": r.labels }),
        )?;
        reply["owner"] = json!(owner);
        reply
    };
    let mandate = reply["mandate"]
        .as_str()
        .ok_or_else(|| anyhow!("no mandate in reply"))?;
    let labels: Vec<String> = reply["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect();
    let config = RunnerConfig {
        server: server.to_owned(),
        name: r.name.to_owned(),
        mandate: mandate.to_owned(),
        labels: labels.clone(),
        ephemeral: reply["ephemeral"].as_bool().unwrap_or(false),
    };
    let path = save_config(&config)?;
    let owner = reply["owner"].as_str().unwrap_or_default().to_owned();
    let mut text = format!(
        "Registered runner {} for {owner} (labels: {}).\n",
        r.name,
        if labels.is_empty() {
            "none".to_owned()
        } else {
            labels.join(", ")
        }
    );
    let mut service = Value::Null;
    if r.install {
        let installed = install(r.name)?;
        text.push_str(&installed.text);
        service = installed.json;
    } else {
        text.push_str(&format!(
            "Start it with `hb runner start {}`, or as a service with `hb runner install {}`.\n",
            r.name, r.name
        ));
    }
    Ok(Output::new(
        json!({ "runner": r.name, "owner": owner, "labels": labels, "config": path, "ephemeral": config.ephemeral, "service": service }),
        text,
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

// ---------------------------------------------------------------- service

/// Makes a registered runner start with the machine: a systemd unit on
/// Linux, a launchd agent on macOS, a scheduled task on Windows.
pub fn install(name: &str) -> Result<Output> {
    load_config(name)?;
    let exe = std::env::current_exe()?;
    let config = config_dir()?;
    #[cfg(target_os = "linux")]
    {
        // Root (cloud-init runs first-boot scripts as root, often without
        // USER): a system service. Anyone else: a user service.
        let root = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("Uid:"))
                    .and_then(|u| u.split_whitespace().next().map(|u| u == "0"))
            })
            .unwrap_or_else(|| std::env::var("USER").is_ok_and(|u| u == "root"));
        let unit_name = format!("hb-runner-{name}.service");
        let dir = if root {
            PathBuf::from("/etc/systemd/system")
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                .ok_or_else(|| anyhow!("no home directory"))?
                .join("systemd/user")
        };
        std::fs::create_dir_all(&dir)?;
        let unit = format!(
            "[Unit]\nDescription=Hebra runner {name}\nAfter=network-online.target\nWants=network-online.target\n\n\
             [Service]\nExecStart={} runner start {name}\nEnvironment=HB_CONFIG_DIR={}\nRestart=always\nRestartSec=5\n\n\
             [Install]\nWantedBy={}\n",
            exe.display(),
            config.display(),
            if root {
                "multi-user.target"
            } else {
                "default.target"
            }
        );
        let path = dir.join(&unit_name);
        std::fs::write(&path, unit)?;
        let mut args = vec!["enable", "--now", unit_name.as_str()];
        if !root {
            args.insert(0, "--user");
        }
        let started = Command::new("systemctl")
            .args(&args)
            .status()
            .is_ok_and(|s| s.success());
        return Ok(Output::new(
            json!({ "kind": "systemd", "path": path, "started": started }),
            format!(
                "Installed {}{}\n",
                path.display(),
                if started {
                    " and started it.".to_owned()
                } else {
                    format!(
                        "; start it with `systemctl {}enable --now {unit_name}`.",
                        if root { "" } else { "--user " }
                    )
                }
            ),
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("no home directory"))?;
        let label = format!("dev.hebra.runner.{name}");
        let dir = PathBuf::from(home).join("Library/LaunchAgents");
        std::fs::create_dir_all(&dir)?;
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n<key>Label</key><string>{label}</string>\n\
             <key>ProgramArguments</key><array><string>{}</string><string>runner</string><string>start</string><string>{name}</string></array>\n\
             <key>EnvironmentVariables</key><dict><key>HB_CONFIG_DIR</key><string>{}</string></dict>\n\
             <key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n</dict></plist>\n",
            exe.display(),
            config.display()
        );
        let path = dir.join(format!("{label}.plist"));
        std::fs::write(&path, plist)?;
        let started = Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        return Ok(Output::new(
            json!({ "kind": "launchd", "path": path, "started": started }),
            format!(
                "Installed {}{}\n",
                path.display(),
                if started { " and started it." } else { "." }
            ),
        ));
    }
    #[cfg(windows)]
    {
        let task = format!("Hebra runner {name}");
        let action = format!("\"{}\" runner start {name}", exe.display());
        let created = Command::new("schtasks")
            .args([
                "/Create", "/F", "/SC", "ONLOGON", "/TN", &task, "/TR", &action,
            ])
            .status()
            .is_ok_and(|s| s.success());
        if !created {
            bail!("could not create the scheduled task (run from an elevated prompt?)");
        }
        let started = Command::new("schtasks")
            .args(["/Run", "/TN", &task])
            .status()
            .is_ok_and(|s| s.success());
        let _ = config;
        return Ok(Output::new(
            json!({ "kind": "scheduled-task", "task": task, "started": started }),
            format!(
                "Installed the scheduled task \"{task}\" (at sign-in){}\n",
                if started { " and started it." } else { "." }
            ),
        ));
    }
    #[allow(unreachable_code)]
    {
        let _ = (exe, config);
        bail!("installing a service is not supported on this system; run `hb runner start {name}`")
    }
}

// ------------------------------------------------------------------- loop

/// `hb runner start`: claims and runs checks until stopped (or one run
/// with `once`; ephemeral runners also remove themselves after it).
pub fn start(name: &str, once: bool, ephemeral: bool) -> Result<Output> {
    let config = load_config(name)?;
    let ephemeral = ephemeral || config.ephemeral;
    let once = once || ephemeral;
    let client = Client::new(&config.server, Some(config.mandate.clone()));
    eprintln!("runner {name}: waiting for checks on {}", config.server);
    let mut done = 0u64;
    let state = loop {
        let claim = match client.post("/runner/claim", &json!({ "version": VERSION })) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("runner {name}: {e:#}; retrying");
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
        };
        if claim["run"].is_null() {
            if once && !ephemeral {
                return Ok(Output::new(
                    json!({ "ran": done }),
                    format!("No checks waiting. Ran {done}.\n"),
                ));
            }
            std::thread::sleep(Duration::from_secs(if ephemeral { 1 } else { 3 }));
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
        let job = Job {
            client: &client,
            runner: name,
            repo: &repo,
            run,
            secrets: claim["secrets"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
            inputs: claim["artifacts"].as_array().cloned().unwrap_or_default(),
        };
        let (state, exit_code, summary) = match execute(&job) {
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
            break state;
        }
    };
    if ephemeral {
        let _ = client.send("DELETE", "/runner/self", &json!({}));
        if let Ok(dir) = runners_dir() {
            let _ = std::fs::remove_file(dir.join(format!("{name}.json")));
        }
    }
    Ok(Output::new(
        json!({ "ran": done, "state": state, "ephemeral": ephemeral }),
        format!("Ran {done} check(s): {state}.\n"),
    ))
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
    store.batch(|| transport::store_all(&store, &objects))?;
    let id = ObjectId::parse(commit)?;
    let c: Commit = store.read_json(COMMIT, &id)?;
    let tree: Tree = store.read_tree(&c.tree)?;
    let work = dir.join("work");
    std::fs::create_dir_all(&work)?;
    // The same checkout as a workspace: path checks, modes and symlinks.
    wc::checkout(&work, &store, &Tree::default(), &tree)?;
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
    masks: Vec<String>,
}

impl Reporter {
    fn start(
        client: Arc<Client>,
        id: &str,
        masks: Vec<String>,
    ) -> (Self, std::thread::JoinHandle<()>) {
        let reporter = Self {
            client,
            id: id.to_owned(),
            buffer: Arc::new(Mutex::new(String::new())),
            cancelled: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            masks,
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
        let mut masked = text.to_owned();
        for secret in &self.masks {
            masked = masked.replace(secret.as_str(), "***");
        }
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_str(&masked);
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

/// The variables every run starts from: a few of the machine's, `CI`, and
/// the `HEBRA_*` details of the run.
fn base_env(run: &Value, repo: &str) -> Vec<(String, String)> {
    let mut env = Vec::new();
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
        "DOCKER_HOST",
    ] {
        if let Ok(v) = std::env::var(key) {
            env.push((key.to_owned(), v));
        }
    }
    env.push(("CI".to_owned(), "true".to_owned()));
    env.push(("HEBRA_REPO".to_owned(), repo.to_owned()));
    for (key, field) in [
        ("HEBRA_COMMIT", "commit"),
        ("HEBRA_CHECK", "name"),
        ("HEBRA_RUN", "id"),
    ] {
        env.push((
            key.to_owned(),
            run[field].as_str().unwrap_or_default().to_owned(),
        ));
    }
    env
}

/// A command under the platform shell, with a clean environment.
fn shell(command: &str, dir: &Path, env: &[(String, String)]) -> Command {
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
    for (k, v) in env {
        cmd.env(k, v);
    }
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

/// One claimed run and what came with it.
struct Job<'a> {
    client: &'a Client,
    runner: &'a str,
    repo: &'a str,
    run: &'a Value,
    secrets: BTreeMap<String, String>,
    inputs: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct Step {
    name: Option<String>,
    run: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(rename = "if")]
    when: Option<String>,
    workdir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Service {
    name: String,
    image: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CacheEntry {
    key: String,
    paths: Vec<String>,
}

/// Where commands run: this machine, or a container.
enum Executor {
    Host,
    Docker(Container),
}

/// A run's container, its services and their network; removed on drop.
struct Container {
    name: String,
    network: String,
    services: Vec<String>,
}

impl Drop for Container {
    fn drop(&mut self) {
        let mut names = vec![self.name.clone()];
        names.extend(self.services.iter().cloned());
        let _ = Command::new("docker")
            .args(["rm", "-f"])
            .args(&names)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("docker")
            .args(["network", "rm", &self.network])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn docker(args: &[&str], log: &dyn Fn(&str)) -> Result<String> {
    let out = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("running docker (is it installed and running on this runner?)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        log(&format!("docker {}: {err}\n", args.first().unwrap_or(&"")));
        bail!("docker {}: {err}", args.first().unwrap_or(&""));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

impl Container {
    fn start(
        id: &str,
        image: &str,
        services: &[Service],
        work: &Path,
        log: &dyn Fn(&str),
    ) -> Result<Self> {
        let short: String = id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(16)
            .collect();
        let network = format!("hb-{short}");
        let mut container = Self {
            name: format!("hb-{short}"),
            network: network.clone(),
            services: Vec::new(),
        };
        docker(&["network", "create", &network], log)?;
        for service in services {
            let name = format!("hb-{short}-{}", service.name);
            log(&format!(
                "$ docker run {} (service {})\n",
                service.image, service.name
            ));
            let mut args: Vec<String> = vec![
                "run".into(),
                "-d".into(),
                "--name".into(),
                name.clone(),
                "--network".into(),
                network.clone(),
                "--network-alias".into(),
                service.name.clone(),
            ];
            for (k, v) in &service.env {
                args.push("-e".into());
                args.push(format!("{k}={v}"));
            }
            args.push(service.image.clone());
            let view: Vec<&str> = args.iter().map(String::as_str).collect();
            docker(&view, log)?;
            container.services.push(name);
        }
        log(&format!("$ docker run {image}\n"));
        let mount = format!("{}:/work", work.display());
        docker(
            &[
                "run",
                "-d",
                "--name",
                &container.name,
                "--network",
                &network,
                "-v",
                &mount,
                "-w",
                "/work",
                "--entrypoint",
                "sh",
                image,
                "-c",
                "sleep 86400",
            ],
            log,
        )?;
        Ok(container)
    }
}

/// Executes one claimed run: (state, exit code, summary).
fn execute(job: &Job<'_>) -> Result<(String, Option<i64>, String)> {
    let run = job.run;
    let id = run["id"].as_str().unwrap_or_default();
    let commit = run["commit"]
        .as_str()
        .ok_or_else(|| anyhow!("run without commit"))?;
    let spec = &run["spec"];
    let base = spec["base"].as_str();
    let dir = tempfile::tempdir()?;
    let client = Arc::new(Client::new(
        &job.client.base,
        job.client.token().map(str::to_owned),
    ));
    let masks: Vec<String> = job
        .secrets
        .values()
        .filter(|v| v.len() >= MIN_MASKED)
        .cloned()
        .collect();
    let (reporter, handle) = Reporter::start(Arc::clone(&client), id, masks);
    let result = (|| {
        reporter.log(&format!(
            "$ hb checkout {}\n",
            commit.get(..12).unwrap_or(commit)
        ));
        let (store, c) = checkout(&client, job.repo, commit, base, dir.path())?;
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
            _ => run_command(job, &client, &reporter, spec, &work),
        }
    })();
    if let Err(e) = &result {
        reporter.log(&format!("\nerror: {e:#}\n"));
    }
    reporter.stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    result
}

fn run_command(
    job: &Job<'_>,
    client: &Client,
    reporter: &Reporter,
    spec: &Value,
    work: &Path,
) -> Result<(String, Option<i64>, String)> {
    let started = Instant::now();
    let timeout = Duration::from_secs(spec["timeout_secs"].as_u64().unwrap_or(1200));
    let log = |t: &str| reporter.log(t);
    let mut steps: Vec<Step> = Vec::new();
    if let Some(run) = spec["run"].as_str() {
        steps.push(Step {
            name: None,
            run: run.to_owned(),
            env: BTreeMap::new(),
            when: None,
            workdir: None,
        });
    }
    steps.extend(serde_json::from_value::<Vec<Step>>(spec["steps"].clone()).unwrap_or_default());
    if steps.is_empty() {
        bail!("check without command");
    }
    let mut env = base_env(job.run, job.repo);
    if let Some(vars) = spec["env"].as_object() {
        for (k, v) in vars {
            env.push((k.clone(), v.as_str().unwrap_or_default().to_owned()));
        }
    }
    for (k, v) in &job.secrets {
        env.push((k.clone(), v.clone()));
    }

    // Artifacts of the checks this one needs.
    if !job.inputs.is_empty() {
        let into = work.join(".artifacts");
        for a in &job.inputs {
            let (Some(run), Some(check), Some(name)) =
                (a["run"].as_str(), a["check"].as_str(), a["name"].as_str())
            else {
                continue;
            };
            let dir = into.join(safe_name(check));
            std::fs::create_dir_all(&dir)?;
            log(&format!("$ hb artifact get {check}/{name}\n"));
            let bytes = client.get_raw(&format!(
                "/repos/{}/checks/{run}/artifacts/{name}",
                job.repo
            ))?;
            std::fs::write(dir.join(name), bytes)?;
        }
        env.push(("HEBRA_ARTIFACTS".to_owned(), ".artifacts".to_owned()));
    }

    let caches: Vec<CacheEntry> = serde_json::from_value(spec["cache"].clone()).unwrap_or_default();
    let cache_root = cache_dir(job.runner)?.join(safe_name(job.repo));
    for c in &caches {
        let slot = cache_root.join(
            blake3::hash(c.key.as_bytes())
                .to_hex()
                .get(..32)
                .unwrap_or_default(),
        );
        for (i, path) in c.paths.iter().enumerate() {
            let Some(target) = inside(work, path) else {
                log(&format!(
                    "cache path `{path}` is outside the checkout; ignored\n"
                ));
                continue;
            };
            let saved = slot.join(i.to_string());
            if saved.exists() {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if std::fs::rename(&saved, &target).is_ok() {
                    log(&format!("cache hit: {path} ({})\n", c.key));
                }
            } else {
                log(&format!("cache miss: {path} ({})\n", c.key));
            }
        }
    }

    let executor = match spec["image"].as_str() {
        Some(image) => {
            let services: Vec<Service> =
                serde_json::from_value(spec["services"].clone()).unwrap_or_default();
            Executor::Docker(Container::start(
                job.run["id"].as_str().unwrap_or("run"),
                image,
                &services,
                work,
                &log,
            )?)
        }
        None => Executor::Host,
    };

    let mut failed: Option<(Option<i32>, String)> = None;
    for (n, step) in steps.iter().enumerate() {
        let wanted = match step.when.as_deref() {
            Some("always") => true,
            Some("failure") => failed.is_some(),
            _ => failed.is_none(),
        };
        let title = step
            .name
            .clone()
            .unwrap_or_else(|| format!("step {}", n + 1));
        if !wanted {
            log(&format!("\n▷ {title} (skipped)\n"));
            continue;
        }
        if steps.len() > 1 {
            log(&format!("\n▶ {title}\n"));
        }
        log(&format!("$ {}\n", step.run));
        let mut step_env = env.clone();
        for (k, v) in &step.env {
            step_env.push((k.clone(), v.clone()));
        }
        let left = timeout.saturating_sub(started.elapsed());
        if left.is_zero() {
            failed = Some((None, format!("timed out after {}s", timeout.as_secs())));
            break;
        }
        let child = match &executor {
            Executor::Host => {
                let dir = match &step.workdir {
                    Some(w) => inside(work, w)
                        .ok_or_else(|| anyhow!("workdir `{w}` is outside the checkout"))?,
                    None => work.to_path_buf(),
                };
                shell(&step.run, &dir, &step_env)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .with_context(|| format!("starting `{}`", step.run))?
            }
            Executor::Docker(container) => {
                // Variables go through a file: never on a command line.
                let env_file = tempfile::NamedTempFile::new()?;
                let mut text = String::new();
                for (k, v) in step_env.iter().filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "PATH"
                            | "HOME"
                            | "USERPROFILE"
                            | "TEMP"
                            | "TMP"
                            | "TMPDIR"
                            | "SYSTEMROOT"
                            | "COMSPEC"
                            | "DOCKER_HOST"
                    )
                }) {
                    if !v.contains('\n') {
                        text.push_str(&format!("{k}={v}\n"));
                    }
                }
                std::fs::write(env_file.path(), text)?;
                let workdir = format!(
                    "/work/{}",
                    step.workdir
                        .as_deref()
                        .unwrap_or("")
                        .trim_start_matches('/')
                );
                let child = Command::new("docker")
                    .args(["exec", "--env-file"])
                    .arg(env_file.path())
                    .args([
                        "-w",
                        workdir.trim_end_matches('/'),
                        &container.name,
                        "sh",
                        "-c",
                        &step.run,
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("running docker exec")?;
                // The file is read when the process starts.
                std::thread::sleep(Duration::from_millis(300));
                drop(env_file);
                child
            }
        };
        let (code, reason) = supervise(child, left, &reporter.cancelled, &log)?;
        if code != Some(0) && failed.is_none() {
            failed = Some((
                code,
                if reason.is_empty() {
                    format!("{title} failed")
                } else {
                    reason
                },
            ));
        }
    }
    drop(executor);

    let success = failed.is_none();
    if success {
        for c in &caches {
            let slot = cache_root.join(
                blake3::hash(c.key.as_bytes())
                    .to_hex()
                    .get(..32)
                    .unwrap_or_default(),
            );
            for (i, path) in c.paths.iter().enumerate() {
                let Some(source) = inside(work, path).filter(|p| p.exists()) else {
                    continue;
                };
                let saved = slot.join(i.to_string());
                let _ = std::fs::remove_dir_all(&saved);
                let _ = std::fs::remove_file(&saved);
                std::fs::create_dir_all(&slot)?;
                if std::fs::rename(&source, &saved).is_ok() {
                    log(&format!("cache saved: {path} ({})\n", c.key));
                }
            }
        }
        let patterns: Vec<String> =
            serde_json::from_value(spec["artifacts"].clone()).unwrap_or_default();
        for pattern in &patterns {
            upload_artifacts(client, job, work, pattern, &log)?;
        }
    }
    Ok(match failed {
        None => ("success".to_owned(), Some(0), "passed".to_owned()),
        Some((Some(c), _)) => (
            "failure".to_owned(),
            Some(i64::from(c)),
            format!("exited with {c}"),
        ),
        Some((None, reason)) => ("failure".to_owned(), None, reason),
    })
}

/// `name` as a file name: letters, digits, `-`, `_`, `.`, `+`.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_start_matches('.')
        .to_owned()
}

/// A path under `work`, or `None` if it would leave it.
fn inside(work: &Path, path: &str) -> Option<PathBuf> {
    let mut out = work.to_path_buf();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => out.push(p),
        }
    }
    (out != work).then_some(out)
}

/// `*` within a segment, `**` across segments, `?` one character.
fn glob(pattern: &str, path: &str) -> bool {
    fn segment(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => segment(&p[1..], s) || (!s.is_empty() && segment(p, &s[1..])),
            (Some(b'?'), Some(_)) => segment(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => segment(&p[1..], &s[1..]),
            _ => false,
        }
    }
    fn parts(p: &[&str], s: &[&str]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(&"**"), _) => parts(&p[1..], s) || (!s.is_empty() && parts(p, &s[1..])),
            (Some(a), Some(b)) => segment(a.as_bytes(), b.as_bytes()) && parts(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let p: Vec<&str> = pattern.split('/').filter(|x| !x.is_empty()).collect();
    let s: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    parts(&p, &s)
}

fn files_under(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel == ".hebra-objects" || rel.starts_with(".artifacts") {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            files_under(root, &path, out);
        } else {
            out.push(rel);
        }
    }
}

/// Uploads what one `artifacts` pattern matches: a single file as it is,
/// several as a `.tar.gz` named after the pattern.
fn upload_artifacts(
    client: &Client,
    job: &Job<'_>,
    work: &Path,
    pattern: &str,
    log: &dyn Fn(&str),
) -> Result<()> {
    let mut all = Vec::new();
    files_under(work, work, &mut all);
    let matched: Vec<String> = all
        .into_iter()
        .filter(|p| {
            glob(pattern, p) || p.starts_with(&format!("{}/", pattern.trim_end_matches('/')))
        })
        .collect();
    let id = job.run["id"].as_str().unwrap_or_default();
    match matched.as_slice() {
        [] => {
            log(&format!("artifacts: nothing matches `{pattern}`\n"));
            Ok(())
        }
        [one] => {
            let name = safe_name(one.rsplit('/').next().unwrap_or(one));
            let bytes = std::fs::read(work.join(one))?;
            log(&format!("artifact: {name} ({} bytes)\n", bytes.len()));
            client.post_raw(&format!("/runner/runs/{id}/artifacts?name={name}"), bytes)?;
            Ok(())
        }
        many => {
            let mut files = Vec::new();
            for p in many {
                let full = work.join(p);
                let executable = is_executable(&full);
                files.push((p.clone(), std::fs::read(&full)?, executable));
            }
            let name = format!(
                "{}.tar.gz",
                safe_name(&pattern.replace(['*', '/'], "_")).trim_matches('_')
            );
            let bytes = hebra_core::archive::tar_gz(&files, 0)?;
            log(&format!(
                "artifact: {name} ({} files, {} bytes)\n",
                many.len(),
                bytes.len()
            ));
            client.post_raw(&format!("/runner/runs/{id}/artifacts?name={name}"), bytes)?;
            Ok(())
        }
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// What the verifier uses to write to the run log.
pub type Sink<'a> = dyn Fn(&str) + 'a;

fn reporter_sink(reporter: &Reporter) -> impl Fn(&str) + '_ {
    move |t: &str| reporter.log(t)
}

/// Runs one shell command for the verifier, with a time limit; returns
/// exit code and output (truncated).
pub fn tool_run(command: &str, work: &Path, timeout: Duration) -> Result<String> {
    let env = base_env(&Value::Null, "");
    let child = shell(command, work, &env)
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
    fn long_output_is_truncated_in_the_middle() {
        let long = "x".repeat(TOOL_OUTPUT_LIMIT * 2);
        let t = truncate(&long);
        assert!(t.len() < long.len());
        assert!(t.contains("omitted"));
    }

    #[test]
    fn globs_and_paths() {
        assert!(glob("dist/**", "dist/a/b.js"));
        assert!(glob("target/release/hb*", "target/release/hb.exe"));
        assert!(!glob("target/release/hb*", "target/debug/hb"));
        assert!(glob("**/*.xml", "reports/unit/a.xml"));
        assert!(inside(Path::new("/w"), "../etc").is_none());
        assert_eq!(
            inside(Path::new("/w"), "a/b"),
            Some(PathBuf::from("/w/a/b"))
        );
        assert_eq!(safe_name("acme/app"), "acme_app");
    }
}
