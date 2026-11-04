//! Releases on a tag with files, and webhooks that receive signed events.

mod common;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use serde_json::{Value, json};

type Received = Arc<Mutex<Vec<(String, String, String)>>>;

async fn receive(State(r): State<Received>, headers: HeaderMap, body: String) -> &'static str {
    let get = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    r.lock()
        .unwrap()
        .push((get("x-hebra-event"), get("x-hebra-signature-256"), body));
    "ok"
}

fn receiver() -> (String, Received) {
    let got: Received = Arc::default();
    let shared = Arc::clone(&got);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let app = Router::new()
                .route("/hook", post(receive))
                .with_state(shared);
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(l.local_addr().unwrap()).unwrap();
            axum::serve(l, app).await.unwrap();
        });
    });
    (format!("http://{}/hook", rx.recv().unwrap()), got)
}

fn hmac_sha256(key: &[u8], body: &[u8]) -> String {
    use sha2::Digest;
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha2::Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = sha2::Sha256::digest([pad(0x36), body.to_vec()].concat());
    let outer = sha2::Sha256::digest([pad(0x5c), inner.to_vec()].concat());
    format!(
        "sha256={}",
        outer.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

#[test]
fn releases_carry_files_and_webhooks_get_signed_events() {
    let env = common::start_server();
    let (url, got) = receiver();
    let (status, hook) = env.api(
        "POST",
        &format!("/repos/{}/webhooks", env.repo),
        &env.human,
        Some(json!({ "url": url, "events": ["push", "release"] })),
    );
    assert_eq!(status, 200, "{hook}");
    let secret = hook["secret"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("whsec_"));

    let ana = env.clone_dir("ana", &env.human);
    fs::write(ana.join("app.rs"), "fn main() {}\n").unwrap();
    env.ok(&ana, &["commit", "-m", "first"], &[]);
    env.ok(&ana, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    env.ok(&ana, &["tag", "-a", "v1.0", "-m", "first release"], &[]);
    env.ok(&ana, &["push", "--tags"], &[]);

    let (status, rel) = env.api(
        "POST",
        &format!("/repos/{}/releases", env.repo),
        &env.human,
        Some(json!({ "tag": "v1.0", "title": "One", "notes": "First release." })),
    );
    assert_eq!(status, 200, "{rel}");
    let (status, _) = env.api(
        "POST",
        &format!("/repos/{}/releases", env.repo),
        &env.human,
        Some(json!({ "tag": "nope" })),
    );
    assert_eq!(status, 400);

    // A file on it, uploaded and downloaded.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let asset = format!(
        "{}/api/v1/repos/{}/releases/v1.0/assets/app-linux.tar.gz",
        env.server, env.repo
    );
    let resp = agent
        .put(&asset)
        .header("Authorization", &format!("Bearer {}", env.human))
        .header("Content-Type", "application/gzip")
        .send(&b"binary bytes"[..])
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let mut resp = agent
        .get(&asset)
        .header("Authorization", &format!("Bearer {}", env.human))
        .call()
        .unwrap();
    assert_eq!(resp.body_mut().read_to_vec().unwrap(), b"binary bytes");
    let (_, shown) = env.api(
        "GET",
        &format!("/repos/{}/releases/v1.0", env.repo),
        &env.human,
        None,
    );
    assert_eq!(shown["assets"][0]["downloads"], 1, "{shown}");
    assert_eq!(shown["notes"], "First release.");

    // The webhook saw the push and the release, signed.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let events: Vec<String> = got
            .lock()
            .unwrap()
            .iter()
            .map(|(e, ..)| e.clone())
            .collect();
        if events.contains(&"push".to_owned()) && events.contains(&"release".to_owned()) {
            break;
        }
        assert!(Instant::now() < deadline, "events so far: {events:?}");
        std::thread::sleep(Duration::from_millis(200));
    }
    for (event, signature, body) in got.lock().unwrap().iter() {
        assert_eq!(
            *signature,
            hmac_sha256(secret.as_bytes(), body.as_bytes()),
            "{event}"
        );
        let v: Value = serde_json::from_str(body).unwrap();
        assert_eq!(v["repo"], env.repo);
    }
    let (_, deliveries) = env.api(
        "GET",
        &format!(
            "/repos/{}/webhooks/{}/deliveries",
            env.repo,
            hook["id"].as_str().unwrap()
        ),
        &env.human,
        None,
    );
    assert!(
        deliveries["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["status"] == 200),
        "{deliveries}"
    );
    let (status, ping) = env.api(
        "POST",
        &format!(
            "/repos/{}/webhooks/{}/ping",
            env.repo,
            hook["id"].as_str().unwrap()
        ),
        &env.human,
        Some(json!({})),
    );
    assert_eq!((status, ping["status"].clone()), (200, json!(200)));
}
