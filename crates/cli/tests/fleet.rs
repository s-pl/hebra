//! The fleet orchestrator against stand-ins for the Hetzner Cloud,
//! DigitalOcean and Vultr APIs, each answering in its provider's shapes: it
//! reads types and prices, creates a machine whose first-boot script
//! registers it as a runner, sees it connect, deletes machines it does not
//! know, and deletes its own when the fleet is switched off.

mod common;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct Machine {
    id: String,
    name: String,
    script: String,
    tags: Vec<String>,
    deleted: bool,
}

#[derive(Default)]
struct Cloud {
    next: u64,
    machines: Vec<Machine>,
    /// Every Authorization header the API saw.
    auth: Vec<String>,
}

type Shared = Arc<Mutex<Cloud>>;

fn seen(s: &Shared, headers: &HeaderMap) {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    s.lock().unwrap().auth.push(auth);
}

fn add(s: &Shared, name: &str, script: String, tags: Vec<String>, id: Option<String>) -> String {
    let mut c = s.lock().unwrap();
    c.next += 1;
    let id = id.unwrap_or_else(|| c.next.to_string());
    c.machines.push(Machine {
        id: id.clone(),
        name: name.to_owned(),
        script,
        tags,
        deleted: false,
    });
    id
}

fn live(s: &Shared, tag: impl Fn(&Machine) -> bool) -> Vec<Machine> {
    s.lock()
        .unwrap()
        .machines
        .iter()
        .filter(|m| !m.deleted && tag(m))
        .cloned()
        .collect()
}

fn remove(s: &Shared, id: &str) -> StatusCode {
    let mut c = s.lock().unwrap();
    match c.machines.iter_mut().find(|m| m.id == id && !m.deleted) {
        Some(m) => {
            m.deleted = true;
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

// ------------------------------------------------------------- Hetzner

async fn hz_types(State(s): State<Shared>, headers: HeaderMap) -> axum::Json<Value> {
    seen(&s, &headers);
    let t = |name: &str, cores: u64, memory: f64, price: &str| {
        json!({
            "name": name, "cores": cores, "memory": memory, "architecture": "x86", "deprecated": false,
            "prices": [{ "location": "fsn1", "price_hourly": { "net": price, "gross": price } }]
        })
    };
    axum::Json(
        json!({ "server_types": [t("cx22", 2, 4.0, "0.0060"), t("cx42", 8, 16.0, "0.0220")] }),
    )
}

async fn hz_create(
    State(s): State<Shared>,
    headers: HeaderMap,
    axum::Json(b): axum::Json<Value>,
) -> axum::Json<Value> {
    seen(&s, &headers);
    let tag = b["labels"]["hebra-fleet"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let id = add(
        &s,
        b["name"].as_str().unwrap_or_default(),
        b["user_data"].as_str().unwrap_or_default().to_owned(),
        vec![tag],
        None,
    );
    let id: u64 = id.parse().unwrap();
    axum::Json(json!({ "server": { "id": id, "public_net": { "ipv4": { "ip": "203.0.113.7" } } } }))
}

async fn hz_list(
    State(s): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<Value> {
    seen(&s, &headers);
    let want = q
        .get("label_selector")
        .and_then(|l| l.strip_prefix("hebra-fleet="))
        .unwrap_or("")
        .to_owned();
    let servers: Vec<Value> = live(&s, |m| m.tags.contains(&want))
        .iter()
        .map(|m| json!({ "id": m.id.parse::<u64>().unwrap(), "name": m.name, "status": "running" }))
        .collect();
    axum::Json(json!({ "servers": servers }))
}

async fn hz_delete(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> StatusCode {
    seen(&s, &headers);
    match remove(&s, &id) {
        StatusCode::NOT_FOUND => StatusCode::NOT_FOUND,
        _ => StatusCode::OK,
    }
}

// --------------------------------------------------------- DigitalOcean

async fn do_sizes(State(s): State<Shared>, headers: HeaderMap) -> axum::Json<Value> {
    seen(&s, &headers);
    axum::Json(json!({ "sizes": [
        { "slug": "s-2vcpu-4gb", "vcpus": 2, "memory": 4096, "price_hourly": 0.03571, "price_monthly": 24.0,
          "regions": ["fra1", "ams3"], "available": true },
        { "slug": "c-8", "vcpus": 8, "memory": 16384, "price_hourly": 0.25, "price_monthly": 168.0,
          "regions": ["fra1"], "available": true },
        { "slug": "s-1vcpu-1gb", "vcpus": 1, "memory": 1024, "price_hourly": 0.00893, "price_monthly": 6.0,
          "regions": ["nyc1"], "available": true },
    ] }))
}

async fn do_create(
    State(s): State<Shared>,
    headers: HeaderMap,
    axum::Json(b): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    seen(&s, &headers);
    let tags = b["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let name = b["name"].as_str().unwrap_or_default().to_owned();
    let id = add(
        &s,
        &name,
        b["user_data"].as_str().unwrap_or_default().to_owned(),
        tags,
        None,
    );
    let id: u64 = id.parse().unwrap();
    (
        StatusCode::ACCEPTED,
        axum::Json(json!({ "droplet": { "id": id, "name": name, "status": "new" } })),
    )
}

async fn do_list(
    State(s): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<Value> {
    seen(&s, &headers);
    let want = q.get("tag_name").cloned().unwrap_or_default();
    let droplets: Vec<Value> = live(&s, |m| m.tags.contains(&want))
        .iter()
        .map(|m| json!({ "id": m.id.parse::<u64>().unwrap(), "name": m.name, "status": "active" }))
        .collect();
    axum::Json(json!({ "droplets": droplets }))
}

async fn do_delete(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> StatusCode {
    seen(&s, &headers);
    remove(&s, &id)
}

// ---------------------------------------------------------------- Vultr

fn unbase64(text: &str) -> String {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    for chunk in text.trim_end_matches('=').as_bytes().chunks(4) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &c)| n | (u32::from(value(c)) << (18 - 6 * i)));
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    String::from_utf8(out).unwrap()
}

async fn vu_plans(State(s): State<Shared>, headers: HeaderMap) -> axum::Json<Value> {
    seen(&s, &headers);
    axum::Json(json!({ "plans": [
        { "id": "vc2-2c-4gb", "vcpu_count": 2, "ram": 4096, "monthly_cost": 20, "hourly_cost": 0.03, "locations": ["fra", "ams"] },
        { "id": "vc2-8c-32gb", "vcpu_count": 8, "ram": 32768, "monthly_cost": 160, "hourly_cost": 0.238, "locations": ["fra"] },
    ], "meta": { "total": 2 } }))
}

async fn vu_create(
    State(s): State<Shared>,
    headers: HeaderMap,
    axum::Json(b): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    seen(&s, &headers);
    assert!(b["os_id"].is_number(), "{b}");
    let tags = b["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let label = b["label"].as_str().unwrap_or_default().to_owned();
    let script = unbase64(b["user_data"].as_str().unwrap_or_default());
    let id = format!("cb676a46-66fd-4dfb-b839-{:012}", s.lock().unwrap().next + 1);
    add(&s, &label, script, tags, Some(id.clone()));
    (
        StatusCode::ACCEPTED,
        axum::Json(
            json!({ "instance": { "id": id, "label": label, "main_ip": "0.0.0.0", "status": "pending", "power_status": "running" } }),
        ),
    )
}

async fn vu_list(
    State(s): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<Value> {
    seen(&s, &headers);
    let want = q.get("tag").cloned().unwrap_or_default();
    let instances: Vec<Value> = live(&s, |m| m.tags.contains(&want))
        .iter()
        .map(|m| json!({ "id": m.id, "label": m.name, "status": "active", "power_status": "running" }))
        .collect();
    axum::Json(json!({ "instances": instances, "meta": { "total": instances.len() } }))
}

async fn vu_delete(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> StatusCode {
    seen(&s, &headers);
    remove(&s, &id)
}

// -------------------------------------------------------------- harness

fn start_cloud() -> (String, Shared) {
    let state: Shared = Arc::default();
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = Arc::clone(&state);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let app = Router::new()
                .route("/v1/server_types", get(hz_types))
                .route("/v1/servers", get(hz_list).post(hz_create))
                .route("/v1/servers/{id}", delete(hz_delete))
                .route("/v2/sizes", get(do_sizes))
                .route("/v2/droplets", get(do_list).post(do_create))
                .route("/v2/droplets/{id}", delete(do_delete))
                .route("/v2/plans", get(vu_plans))
                .route("/v2/instances", get(vu_list).post(vu_create))
                .route("/v2/instances/{id}", delete(vu_delete))
                .with_state(shared);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    (format!("http://{}", rx.recv().unwrap()), state)
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn quoted_after<'a>(script: &'a str, flag: &str) -> &'a str {
    let rest = script.split(&format!("{flag} '")).nth(1).unwrap();
    rest.split('\'').next().unwrap()
}

struct Dialect {
    provider: &'static str,
    /// The API's base, after the stand-in's address.
    base: &'static str,
    region: &'static str,
    /// The cheapest type in the region, first in the catalog.
    cheapest: &'static str,
    /// The tag a machine of this fleet carries, from the fleet's tag.
    tag: fn(&str) -> String,
    /// Least that one hour of the machine it starts costs (micros).
    least_cost: i64,
}

fn orchestrates(d: &Dialect) {
    let env = common::start_server();
    let (cloud_url, cloud) = start_cloud();
    let ana = env.clone_dir("ana", &env.human);
    fs::create_dir_all(ana.join(".hb")).unwrap();
    fs::write(
        ana.join(".hb/checks.toml"),
        "[[check]]\nname = \"test\"\non = [\"push:main\"]\nlabels = [\"linux\"]\nrun = \"echo from the fleet\"\n",
    )
    .unwrap();
    fs::write(ana.join("app.rs"), "fn main() {}\n").unwrap();
    env.ok(&ana, &["commit", "-m", "first"], &[]);
    env.ok(&ana, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    let fleet = format!("/accounts/{}/fleet", env.ana());
    let api = format!("{cloud_url}{}", d.base);
    let settings = |enabled: bool| {
        json!({
            "enabled": enabled,
            "provider": d.provider,
            "credentials": "test-token",
            "config": {
                "api_url": api, "region": d.region, "goal": "fast", "max_machines": 2,
                "server_url": env.server, "docker": false, "labels": ["linux"]
            }
        })
    };
    let (status, put) = env.api("PUT", &fleet, &env.human, Some(settings(true)));
    assert_eq!(status, 200, "{put}");
    assert_eq!(put["fleet"]["has_credentials"], true);
    let (status, checked) = env.api(
        "POST",
        &format!("{fleet}/check"),
        &env.human,
        Some(json!({})),
    );
    assert_eq!(status, 200, "{checked}");
    assert_eq!(checked["types"][0]["name"], d.cheapest, "{checked}");
    let names: Vec<&str> = checked["types"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(
        names.len(),
        2,
        "only types offered in {}: {names:?}",
        d.region
    );

    // Heddle starts a machine for the queued check.
    wait_for("a machine to be created", || {
        !cloud.lock().unwrap().machines.is_empty()
    });
    let made = cloud.lock().unwrap().machines[0].clone();
    assert!(made.script.starts_with("#!/bin/sh"), "{}", made.script);
    assert!(
        made.script.contains("hb runner register"),
        "{}",
        made.script
    );
    let tag = (d.tag)(env.ana());
    assert!(made.tags.contains(&tag), "{:?} lacks {tag}", made.tags);
    let token = quoted_after(&made.script, "--token").to_owned();
    assert_eq!(quoted_after(&made.script, "--name"), made.name);

    // The machine boots: its first-boot script registers it and it works.
    let home = env.root.join("vps");
    fs::create_dir_all(&home).unwrap();
    env.ok(
        &home,
        &[
            "runner",
            "register",
            "--server",
            &env.server,
            "--token",
            &token,
            "--name",
            &made.name,
            "--machine",
            &made.name,
        ],
        &[],
    );
    env.ok(&home, &["runner", "start", &made.name, "--once"], &[]);
    let runs = env.ok(&ana, &["checks", "list", "main"], &[]);
    assert_eq!(runs["checks"][0]["state"], "success", "{runs}");

    let machine_state = || {
        let (status, v) = env.api("GET", &fleet, &env.human, None);
        assert_eq!(status, 200, "{v}");
        v["machines"][0]["state"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    wait_for("the machine to be ready", || machine_state() == "ready");
    let (_, v) = env.api("GET", &fleet, &env.human, None);
    assert_eq!(
        v["runs"][0]["machine"],
        made.name.as_str(),
        "the timeline knows where it ran: {v}"
    );

    // A machine with the fleet's tag that the server never made is deleted.
    add(&cloud, "stray", String::new(), vec![tag.clone()], None);
    wait_for("the stray machine to be deleted", || {
        cloud
            .lock()
            .unwrap()
            .machines
            .iter()
            .any(|m| m.name == "stray" && m.deleted)
    });

    // Switched off: idle machines go, at the provider too.
    let (status, _) = env.api("PUT", &fleet, &env.human, Some(settings(false)));
    assert_eq!(status, 200);
    wait_for("the machine to be deleted", || machine_state() == "gone");
    assert!(cloud.lock().unwrap().machines.iter().all(|m| m.deleted));
    let (_, v) = env.api("GET", &fleet, &env.human, None);
    let kinds: Vec<&str> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    for kind in ["start", "ready", "orphan", "stop"] {
        assert!(kinds.contains(&kind), "no {kind} event in {kinds:?}");
    }
    assert!(
        v["machines"][0]["cost_micros"].as_i64().unwrap() >= d.least_cost,
        "{v}"
    );

    // Every call carried the token, and only it.
    let auth = cloud.lock().unwrap().auth.clone();
    assert!(!auth.is_empty());
    assert!(auth.iter().all(|a| a == "Bearer test-token"), "{auth:?}");
}

#[test]
fn heddle_runs_machines_on_hetzner_cloud() {
    orchestrates(&Dialect {
        provider: "hetzner",
        base: "/v1",
        region: "fsn1",
        cheapest: "cx22",
        tag: |owner| owner.to_owned(),
        least_cost: 6_000,
    });
}

#[test]
fn heddle_runs_machines_on_digitalocean() {
    orchestrates(&Dialect {
        provider: "digitalocean",
        base: "/v2",
        region: "fra1",
        cheapest: "s-2vcpu-4gb",
        tag: |owner| format!("hebra-fleet-{owner}"),
        // Billed by the second, a minute at least.
        least_cost: 500,
    });
}

#[test]
fn heddle_runs_machines_on_vultr() {
    orchestrates(&Dialect {
        provider: "vultr",
        base: "/v2",
        region: "fra",
        cheapest: "vc2-2c-4gb",
        tag: |owner| format!("hebra-fleet-{owner}"),
        least_cost: 30_000,
    });
}

#[test]
fn fleet_addresses_must_be_public_unless_allowed() {
    // The test server allows private addresses (its stand-ins listen on
    // 127.0.0.1); the rule itself is unit-tested in hebra-server.
    let env = common::start_server();
    let (status, v) = env.api(
        "PUT",
        &format!("/accounts/{}/fleet", env.ana()),
        &env.human,
        Some(
            json!({ "enabled": false, "provider": "hetzner", "config": { "api_url": "ftp://x" } }),
        ),
    );
    assert_eq!(status, 400, "{v}");
}
