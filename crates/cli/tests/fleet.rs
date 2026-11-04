//! The fleet orchestrator against a stand-in for the Hetzner Cloud API: it
//! reads types and prices, creates a machine whose first-boot script
//! registers it as a runner, sees it connect, deletes machines it does not
//! know, and deletes its own when the fleet is switched off.

mod common;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, State};
use axum::routing::{delete, get};
use serde_json::{Value, json};

#[derive(Default)]
struct Cloud {
    next: u64,
    servers: Vec<(u64, String, String, bool)>,
}

type Shared = Arc<Mutex<Cloud>>;

async fn types() -> axum::Json<Value> {
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

async fn create(State(s): State<Shared>, axum::Json(body): axum::Json<Value>) -> axum::Json<Value> {
    let mut c = s.lock().unwrap();
    c.next += 1;
    let id = c.next;
    c.servers.push((
        id,
        body["name"].as_str().unwrap_or_default().to_owned(),
        body["user_data"].as_str().unwrap_or_default().to_owned(),
        false,
    ));
    axum::Json(json!({ "server": { "id": id, "public_net": { "ipv4": { "ip": "203.0.113.7" } } } }))
}

async fn list(State(s): State<Shared>) -> axum::Json<Value> {
    let c = s.lock().unwrap();
    let servers: Vec<Value> = c
        .servers
        .iter()
        .filter(|(.., deleted)| !deleted)
        .map(|(id, name, ..)| json!({ "id": id, "name": name, "status": "running" }))
        .collect();
    axum::Json(json!({ "servers": servers }))
}

async fn remove(State(s): State<Shared>, Path(id): Path<u64>) -> axum::Json<Value> {
    let mut c = s.lock().unwrap();
    for server in &mut c.servers {
        if server.0 == id {
            server.3 = true;
        }
    }
    axum::Json(json!({}))
}

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
                .route("/v1/server_types", get(types))
                .route("/v1/servers", get(list).post(create))
                .route("/v1/servers/{id}", delete(remove))
                .with_state(shared);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    (format!("http://{}/v1", rx.recv().unwrap()), state)
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

#[test]
fn the_orchestrator_starts_registers_and_stops_cloud_machines() {
    let env = common::start_server();
    let (api, cloud) = start_cloud();
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
    let settings = |enabled: bool| {
        json!({
            "enabled": enabled,
            "provider": "hetzner",
            "credentials": "test-token",
            "config": {
                "api_url": api, "region": "fsn1", "goal": "fast", "max_machines": 2,
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
    assert_eq!(checked["types"][0]["name"], "cx22");

    // Heddle starts a machine for the queued check.
    wait_for("a machine to be created", || {
        !cloud.lock().unwrap().servers.is_empty()
    });
    let (name, script) = {
        let c = cloud.lock().unwrap();
        (c.servers[0].1.clone(), c.servers[0].2.clone())
    };
    assert!(script.contains("hb runner register"), "{script}");
    let token = quoted_after(&script, "--token").to_owned();
    assert_eq!(quoted_after(&script, "--name"), name);

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
            &name,
            "--machine",
            &name,
        ],
        &[],
    );
    env.ok(&home, &["runner", "start", &name, "--once"], &[]);
    let runs = env.ok(&ana, &["checks", "list", "main"], &[]);
    assert_eq!(runs["checks"][0]["state"], "success", "{runs}");

    let machine_state = || {
        let (_, v) = env.api("GET", &fleet, &env.human, None);
        v["machines"][0]["state"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    wait_for("the machine to be ready", || machine_state() == "ready");

    // A machine with the fleet's tag that the server never made is deleted.
    {
        let mut c = cloud.lock().unwrap();
        c.next += 1;
        let id = c.next;
        c.servers
            .push((id, "stray".to_owned(), String::new(), false));
    }
    wait_for("the stray machine to be deleted", || {
        cloud
            .lock()
            .unwrap()
            .servers
            .iter()
            .any(|s| s.1 == "stray" && s.3)
    });

    // Switched off: idle machines go, at the provider too.
    let (status, _) = env.api("PUT", &fleet, &env.human, Some(settings(false)));
    assert_eq!(status, 200);
    wait_for("the machine to be deleted", || machine_state() == "gone");
    assert!(cloud.lock().unwrap().servers.iter().all(|s| s.3));
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
        v["machines"][0]["cost_micros"].as_i64().unwrap() >= 6000,
        "{v}"
    );
}
