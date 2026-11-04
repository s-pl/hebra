//! `hb heddle`: Heddle™, build intelligence.
//!
//! `hb heddle simulate` measures policies against a trace (a file, or a
//! made-up team, agent swarm or steady load) on a catalog of machine
//! types, so a policy is proven before it spends money. `hb heddle up`
//! runs the same planner against a server's real queue and starts and
//! stops machines through a provider. Today the provider is Docker on this
//! computer: each container stands in for a virtual machine (its CPUs,
//! memory, boot time and price come from the catalog), registers itself
//! with a one-time token and takes checks until Heddle stops it.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use hebra_heddle::plan::{Heddle, baselines};
use hebra_heddle::sim::{Report, Trace, simulate};
use hebra_heddle::{Action, Catalog, Goal, Job, Machine, MachineType, Params, Policy};
use serde_json::{Value, json};

use crate::client::Client;
use crate::commands::Output;

/// Seconds a check is assumed to take before Heddle has seen it run.
const DEFAULT_SECS: f64 = 120.0;

pub fn catalog(path: Option<&str>) -> Result<Catalog> {
    match path {
        Some(p) => {
            let text = std::fs::read_to_string(p).with_context(|| format!("reading {p}"))?;
            Ok(Catalog::parse(&text)?)
        }
        None => Ok(Catalog::example()),
    }
}

pub struct SimulateArgs<'a> {
    pub trace: Option<&'a str>,
    pub workload: &'a str,
    pub days: u32,
    pub catalog: Option<&'a str>,
    pub goal: &'a str,
    pub time_value: Option<f64>,
    pub max_machines: usize,
    pub seed: u64,
}

/// Parameters for a goal, with an explicit time value on top.
fn params_for(goal: &str, time_value: Option<f64>) -> Result<Params> {
    let goal = Goal::parse(goal).ok_or_else(|| anyhow!("the goal is cheap, balanced or fast"))?;
    let mut params = goal.params();
    if let Some(v) = time_value {
        params.time_value_per_hour = v;
    }
    Ok(params)
}

fn row(r: &Report) -> String {
    format!(
        "{:<40} {:>9.3} {:>8.0}s {:>8.0}s {:>9} {:>7.0}% {:>11.2}\n",
        r.policy,
        r.cost,
        r.wait_p50_secs,
        r.wait_p95_secs,
        r.machines_started,
        r.utilization * 100.0,
        r.total_cost
    )
}

pub fn simulate_cmd(a: &SimulateArgs<'_>) -> Result<Output> {
    let catalog = catalog(a.catalog)?;
    let trace = match a.trace {
        Some(p) => {
            Trace::parse(&std::fs::read_to_string(p).with_context(|| format!("reading {p}"))?)?
        }
        None => Trace::synthetic(a.workload, a.days, a.seed),
    };
    if trace.jobs.is_empty() {
        bail!("the trace has no jobs");
    }
    let params = Params {
        max_machines: a.max_machines,
        arrivals_per_hour: trace.rate_per_hour(),
        ..params_for(a.goal, a.time_value)?
    };
    let mut policies = baselines(&catalog.types);
    policies.retain(|p| p.name() != "heddle");
    policies.push(Box::new(Heddle));
    let reports: Vec<Report> = policies
        .iter()
        .map(|p| simulate(&trace, &catalog.types, p.as_ref(), &params))
        .collect();
    let mut text = format!(
        "{} jobs ({}), {} machine types, goal {}: an hour until results is worth {}, waits over {}s start machines.\n\n{:<40} {:>9} {:>9} {:>9} {:>9} {:>8} {:>11}\n",
        trace.jobs.len(),
        a.trace.unwrap_or(a.workload),
        catalog.types.len(),
        a.goal,
        params.time_value_per_hour,
        params.max_wait_secs,
        "policy",
        "machines",
        "wait p50",
        "wait p95",
        "started",
        "busy",
        "with time"
    );
    for r in &reports {
        text.push_str(&row(r));
    }
    if let Some(h) = reports.iter().find(|r| r.policy == "heddle") {
        text.push_str("\nHeddle used:");
        for (kind, u) in &h.by_type {
            let _ = write!(
                text,
                "  {kind}: {} started, {:.1} h ({:.0}% busy), {:.3}",
                u.started,
                u.hours,
                if u.hours > 0.0 {
                    u.busy_hours / u.hours * 100.0
                } else {
                    0.0
                },
                u.cost
            );
        }
        text.push('\n');
    }
    let best = reports
        .iter()
        .min_by(|x, y| x.total_cost.total_cmp(&y.total_cost))
        .map(|r| r.policy.clone())
        .unwrap_or_default();
    let _ = writeln!(text, "\nBest overall: {best}. Prices are the catalog's.");
    Ok(Output::new(
        json!({ "reports": reports, "best": best }),
        text,
    ))
}

// --------------------------------------------------------------- docker

/// Machines as containers on this computer.
struct Docker {
    owner: String,
    image: String,
    /// What the Docker host has: containers cannot ask for more.
    host_cpus: u32,
    host_memory_gb: u32,
}

/// CPUs and memory (GiB) of the Docker host.
fn docker_host() -> (u32, u32) {
    let info = docker(&["info", "--format", "{{.NCPU}} {{.MemTotal}}"]).unwrap_or_default();
    let mut parts = info.split_whitespace();
    let cpus = parts.next().and_then(|n| n.parse().ok()).unwrap_or(2);
    let memory = parts
        .next()
        .and_then(|n| n.parse::<u64>().ok())
        .map_or(4, |b| u32::try_from(b / (1 << 30)).unwrap_or(4).max(1));
    (cpus, memory)
}

struct Container {
    name: String,
    kind: String,
    created: f64,
    running: bool,
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn docker(args: &[&str]) -> Result<String> {
    let out = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("running docker (is Docker running?)")?;
    if !out.status.success() {
        bail!(
            "docker {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

impl Docker {
    fn list(&self) -> Result<Vec<Container>> {
        let filter = format!("label=hebra.heddle={}", self.owner);
        let ids = docker(&["ps", "-a", "-q", "--filter", &filter])?;
        let ids: Vec<&str> = ids.split_whitespace().collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = vec!["inspect"];
        args.extend(ids.iter().copied());
        let info: Value = serde_json::from_str(&docker(&args)?)?;
        let mut out = Vec::new();
        for c in info.as_array().into_iter().flatten() {
            let created = c["Created"]
                .as_str()
                .and_then(|t| t.parse::<jiff::Timestamp>().ok())
                .map_or(0.0, |t| t.as_millisecond() as f64 / 1000.0);
            out.push(Container {
                name: c["Name"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_owned(),
                kind: c["Config"]["Labels"]["hebra.kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                created,
                running: c["State"]["Running"].as_bool().unwrap_or(false),
            });
        }
        Ok(out)
    }

    fn create(&self, kind: &MachineType, name: &str, server: &str, token: &str) -> Result<()> {
        // A machine bigger than this computer gets all of it.
        let cpus = kind.cpus.clamp(1, self.host_cpus.max(1)).to_string();
        let memory = format!("{}g", kind.memory_gb.clamp(1, self.host_memory_gb.max(1)));
        let owner = format!("hebra.heddle={}", self.owner);
        let kind_label = format!("hebra.kind={}", kind.name);
        let envs = [
            format!("HEBRA_SERVER={server}"),
            format!("HB_RUNNER_TOKEN={token}"),
            format!("HEBRA_RUNNER_NAME={name}"),
            format!("HEBRA_MACHINE={name}"),
            format!("HEBRA_BOOT_DELAY={:.0}", kind.boot_secs),
        ];
        let mut args = vec![
            "run",
            "-d",
            "--name",
            name,
            "--label",
            &owner,
            "--label",
            &kind_label,
            "--cpus",
            &cpus,
            "--memory",
            &memory,
            "--add-host",
            "host.docker.internal:host-gateway",
        ];
        for e in &envs {
            args.push("-e");
            args.push(e);
        }
        args.push(&self.image);
        docker(&args)?;
        Ok(())
    }

    fn destroy(&self, name: &str) -> Result<()> {
        docker(&["rm", "-f", name])?;
        Ok(())
    }
}

/// The server as a container sees it: `localhost` is the container itself.
fn from_container(server: &str) -> String {
    server
        .replace("://localhost", "://host.docker.internal")
        .replace("://127.0.0.1", "://host.docker.internal")
}

pub struct UpArgs<'a> {
    pub server: &'a str,
    pub owner: &'a str,
    pub catalog: Option<&'a str>,
    pub image: &'a str,
    pub max_machines: usize,
    pub max_spend: f64,
    pub goal: &'a str,
    pub time_value: Option<f64>,
    pub dry_run: bool,
    /// Stop after this many seconds (0: run until interrupted).
    pub duration_secs: u64,
}

/// What the loop remembers between rounds.
#[derive(Default)]
struct Memory {
    /// When each queued run was first seen.
    seen: HashMap<String, f64>,
    /// When each machine was first seen idle.
    idle_since: HashMap<String, f64>,
    /// Typical seconds of each check, from insights.
    durations: HashMap<String, f64>,
    insights_at: HashMap<String, Instant>,
}

fn reference_secs(client: &Client, memory: &mut Memory, repo: &str, check: &str) -> f64 {
    let fresh = memory
        .insights_at
        .get(repo)
        .is_some_and(|t| t.elapsed() < Duration::from_secs(60));
    if !fresh {
        memory.insights_at.insert(repo.to_owned(), Instant::now());
        if let Ok(v) = client.get(&format!("/repos/{repo}/insights")) {
            for c in v["checks"].as_array().into_iter().flatten() {
                if let (Some(name), Some(ms)) =
                    (c["name"].as_str(), c["duration_ms"]["p50"].as_f64())
                {
                    memory
                        .durations
                        .insert(format!("{repo}:{name}"), ms / 1000.0);
                }
            }
        }
    }
    let base = check.split(" (").next().unwrap_or(check);
    memory
        .durations
        .get(&format!("{repo}:{base}"))
        .copied()
        .unwrap_or(DEFAULT_SECS)
}

/// One round: read the queue and the machines, plan, act. Returns what
/// it did, as lines.
fn round(
    client: &Client,
    provider: &Docker,
    catalog: &Catalog,
    params: &Params,
    args: &UpArgs<'_>,
    memory: &mut Memory,
) -> Result<(Vec<String>, Value)> {
    let now = now_secs();
    let q = client.get(&format!("/accounts/{}/queue", args.owner))?;
    let containers = provider.list()?;
    let runners: BTreeMap<String, &Value> = q["runners"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["machine"].as_str().or(r["name"].as_str())?.to_owned(), r)))
        .collect();

    let mut lines = Vec::new();
    // Containers that stopped on their own are cleaned up.
    for c in containers.iter().filter(|c| !c.running) {
        lines.push(format!("remove {} (it stopped)", c.name));
        if !args.dry_run {
            let _ = provider.destroy(&c.name);
            let _ = client.send(
                "DELETE",
                &format!("/accounts/{}/runners/{}", args.owner, c.name),
                &json!({}),
            );
        }
    }
    let machines: Vec<Machine> = containers
        .iter()
        .filter(|c| c.running)
        .map(|c| {
            let boot = catalog
                .types
                .iter()
                .find(|t| t.name == c.kind)
                .map_or(30.0, |t| t.boot_secs);
            let runner = runners.get(&c.name);
            let online = runner.is_some_and(|r| r["online"].as_bool().unwrap_or(false));
            let busy = runner.is_some_and(|r| r["busy"].as_bool().unwrap_or(false));
            if busy || !online {
                memory.idle_since.remove(&c.name);
            }
            let idle_since =
                (online && !busy).then(|| *memory.idle_since.entry(c.name.clone()).or_insert(now));
            Machine {
                id: c.name.clone(),
                kind: c.kind.clone(),
                created: c.created,
                ready_at: if online {
                    c.created.min(now)
                } else {
                    (c.created + boot).max(now + 1.0)
                },
                busy_until: busy.then_some(now + DEFAULT_SECS / 2.0),
                idle_since,
            }
        })
        .collect();

    let mut jobs = Vec::new();
    for run in q["queued"].as_array().into_iter().flatten() {
        if run["blocked"].as_bool().unwrap_or(false) {
            continue;
        }
        let id = run["id"].as_str().unwrap_or_default().to_owned();
        let waited = run["waiting_ms"].as_f64().unwrap_or(0.0) / 1000.0;
        let queued_at = *memory.seen.entry(id.clone()).or_insert(now - waited);
        let repo = run["repo"].as_str().unwrap_or_default();
        let check = run["name"].as_str().unwrap_or_default();
        jobs.push(Job {
            id: format!("{repo} {check}"),
            labels: run["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect(),
            reference_secs: reference_secs(client, memory, repo, check),
            queued_at,
        });
    }
    memory.seen.retain(|_, t| now - *t < 3600.0);
    let mut known = params.clone();
    known.recent_arrivals = memory.seen.values().copied().collect();

    let actions = Heddle.plan(now, &catalog.types, &machines, &jobs, &known);
    for action in &actions {
        match action {
            Action::Create { kind, reason, .. } => {
                let Some(t) = catalog.types.iter().find(|t| &t.name == kind) else {
                    continue;
                };
                let mut suffix = [0u8; 4];
                rand::fill(&mut suffix);
                let name = format!(
                    "heddle-{}-{}",
                    t.name.replace('_', "-"),
                    hex::encode(suffix)
                );
                lines.push(format!(
                    "start {name} ({}, {} CPUs): {reason}",
                    t.name, t.cpus
                ));
                if args.dry_run {
                    continue;
                }
                let token = client.post(
                    &format!("/accounts/{}/runner-tokens", args.owner),
                    &json!({ "labels": t.labels, "uses": 1, "ttl_secs": 3600 }),
                )?;
                let token = token["token"]
                    .as_str()
                    .ok_or_else(|| anyhow!("no token in reply"))?;
                provider.create(t, &name, &from_container(args.server), token)?;
            }
            Action::Destroy { id, reason } => {
                lines.push(format!("stop {id}: {reason}"));
                if !args.dry_run {
                    provider.destroy(id)?;
                    let _ = client.send(
                        "DELETE",
                        &format!("/accounts/{}/runners/{id}", args.owner),
                        &json!({}),
                    );
                    memory.idle_since.remove(id);
                }
            }
        }
    }
    let status = json!({
        "queued": jobs.len(),
        "machines": machines.len(),
        "busy": machines.iter().filter(|m| m.busy_until.is_some()).count(),
        "spend_per_hour": machines.iter().filter_map(|m| catalog.types.iter().find(|t| t.name == m.kind)).map(|t| t.price_per_hour).sum::<f64>(),
        "actions": actions,
    });
    Ok((lines, status))
}

/// `hb heddle up`: the orchestrator loop.
pub fn up(args: &UpArgs<'_>) -> Result<Output> {
    let catalog = catalog(args.catalog)?;
    let token = crate::account::credential(args.server)
        .or_else(|| std::env::var("HB_TOKEN").ok())
        .ok_or_else(|| anyhow!("run `hb login {}` first", args.server))?;
    let client = Client::new(args.server, Some(token));
    let (host_cpus, host_memory_gb) = docker_host();
    let provider = Docker {
        owner: args.owner.to_owned(),
        image: args.image.to_owned(),
        host_cpus,
        host_memory_gb,
    };
    let params = Params {
        max_machines: args.max_machines,
        max_spend_per_hour: args.max_spend,
        ..params_for(args.goal, args.time_value)?
    };
    let mut memory = Memory::default();
    let started = Instant::now();
    eprintln!(
        "heddle: {} for {} on {} (Docker, image {}){}",
        if args.dry_run {
            "planning"
        } else {
            "orchestrating"
        },
        args.owner,
        args.server,
        args.image,
        if args.duration_secs > 0 {
            format!(", for {}s", args.duration_secs)
        } else {
            String::new()
        }
    );
    let mut log = Vec::new();
    let mut last = Value::Null;
    loop {
        match round(&client, &provider, &catalog, &params, args, &mut memory) {
            Ok((lines, status)) => {
                for l in &lines {
                    eprintln!("heddle: {l}");
                }
                log.extend(lines);
                last = status;
            }
            Err(e) => eprintln!("heddle: {e:#}"),
        }
        if args.dry_run
            || (args.duration_secs > 0 && started.elapsed().as_secs() >= args.duration_secs)
        {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let text = if log.is_empty() {
        "Nothing to do.\n".to_owned()
    } else {
        log.join("\n") + "\n"
    };
    Ok(Output::new(json!({ "log": log, "status": last }), text))
}

/// `hb heddle down`: stops every machine Heddle started for an owner.
pub fn down(server: &str, owner: &str) -> Result<Output> {
    let provider = Docker {
        owner: owner.to_owned(),
        image: String::new(),
        host_cpus: 1,
        host_memory_gb: 1,
    };
    let client = crate::account::credential(server).map(|t| Client::new(server, Some(t)));
    let mut stopped = Vec::new();
    for c in provider.list()? {
        provider.destroy(&c.name)?;
        if let Some(client) = &client {
            let _ = client.send(
                "DELETE",
                &format!("/accounts/{owner}/runners/{}", c.name),
                &json!({}),
            );
        }
        stopped.push(c.name);
    }
    Ok(Output::new(
        json!({ "stopped": stopped }),
        format!("Stopped {} machine(s).\n", stopped.len()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_reach_the_host() {
        assert_eq!(
            from_container("http://127.0.0.1:4600"),
            "http://host.docker.internal:4600"
        );
        assert_eq!(
            from_container("https://hebra.example.com"),
            "https://hebra.example.com"
        );
    }

    #[test]
    fn simulation_compares_every_policy() {
        let out = simulate_cmd(&SimulateArgs {
            trace: None,
            workload: "steady",
            days: 1,
            catalog: None,
            goal: "balanced",
            time_value: None,
            max_machines: 10,
            seed: 1,
        })
        .unwrap();
        assert_eq!(out.json["reports"].as_array().unwrap().len(), 4);
        assert!(out.text.contains("heddle"));
    }
}
