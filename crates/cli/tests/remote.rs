//! End-to-end: a real hebra-server, two clones, people and agents.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use hebra_core::store::Key;
use hebra_server::{Config, NatsConfig, ObjectsConfig, Server, TokenInfo};
use serde_json::Value;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    url: String,
    repo: String,
    human: String,
    agent: String,
}

/// A server like production: objects encrypted, and, when these variables
/// are set (CI does), metadata in PostgreSQL, objects in S3 and live events
/// over NATS: HEBRA_TEST_DATABASE_URL, HEBRA_TEST_S3_BUCKET, HEBRA_TEST_NATS_URL.
fn config(data: &Path) -> Config {
    let mut config = Config::local(data);
    config.encryption_keys = vec![Key::from_hex(&Key::generate_hex()).unwrap()];
    config.database_url = std::env::var("HEBRA_TEST_DATABASE_URL").ok();
    if let Ok(bucket) = std::env::var("HEBRA_TEST_S3_BUCKET") {
        config.objects = ObjectsConfig::S3 { bucket };
    }
    config.nats = std::env::var("HEBRA_TEST_NATS_URL")
        .ok()
        .map(|url| NatsConfig {
            url,
            ..NatsConfig::default()
        });
    config
}

fn start_server() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("server");
    // Unique names (from the random temp dir), so tests can share a database.
    let unique: String = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let repo = format!("acme-{}", unique.to_lowercase());
    let (tx, rx) = std::sync::mpsc::channel();
    let name = repo.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let server = Server::open(config(&data)).await.unwrap();
            server
                .create_repo(&name, &["main".to_owned()])
                .await
                .unwrap();
            let human = server
                .create_token(
                    &TokenInfo {
                        name: "ana".into(),
                        kind: "human".into(),
                        on_behalf_of: None,
                        allow: vec!["*".into()],
                        admin: true,
                    },
                    None,
                )
                .await
                .unwrap();
            let agent = server
                .create_token(
                    &TokenInfo {
                        name: "claude-code".into(),
                        kind: "agent".into(),
                        on_behalf_of: Some("ana".into()),
                        allow: vec!["agents/claude-code/*".into(), "main".into()],
                        admin: false,
                    },
                    Some(std::time::Duration::from_secs(3600)),
                )
                .await
                .unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send((listener.local_addr().unwrap(), human, agent))
                .unwrap();
            hebra_server::serve(Arc::new(server), listener)
                .await
                .unwrap();
        });
    });
    let (addr, human, agent) = rx.recv().unwrap();
    let root = dir.path().to_path_buf();
    Env {
        _dir: dir,
        root,
        url: format!("http://{addr}/{repo}"),
        repo,
        human,
        agent,
    }
}

fn hb(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hb"));
    cmd.current_dir(cwd)
        .args(args)
        .env_remove("CLAUDECODE")
        .env_remove("HB_TOKEN")
        .env_remove("HB_ACTOR")
        .env("USER", "tester")
        .env("USERNAME", "tester");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn ok(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = hb(cwd, &full, env);
    assert!(
        out.status.success(),
        "hb {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn fail(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = hb(cwd, &full, env);
    assert!(!out.status.success(), "hb {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn clone_dir(env: &Env, name: &str, token: &str) -> PathBuf {
    let dir = env.root.join(name);
    fs::create_dir_all(&dir).unwrap();
    ok(&dir, &["init"], &[]);
    ok(&dir, &["config", "set", "user.name", name], &[]);
    ok(
        &dir,
        &["remote", "add", "origin", &env.url, "--token", token],
        &[],
    );
    dir
}

#[test]
fn push_pull_permissions_and_presence() {
    let env = start_server();

    // Ana publishes main.
    let a = clone_dir(&env, "ana", &env.human);
    fs::write(a.join("app.rs"), "fn main() {}\n").unwrap();
    ok(&a, &["commit", "-m", "first"], &[]);
    let pushed = ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert_eq!(pushed["pushed"][0]["bookmark"], "main");

    // A second machine pulls it and sees the files.
    let b = clone_dir(&env, "luis", &env.human);
    let pulled = ok(&b, &["pull"], &[]);
    assert_eq!(pulled["updated"][0], "main");
    ok(&b, &["new", "main"], &[]);
    assert_eq!(
        fs::read_to_string(b.join("app.rs")).unwrap(),
        "fn main() {}\n"
    );

    // An agent may publish to its own bookmarks but never move main.
    let c = clone_dir(&env, "agent", &env.agent);
    ok(&c, &["pull"], &[]);
    ok(&c, &["new", "main"], &[]);
    fs::write(c.join("oauth.rs"), "// oauth\n").unwrap();
    ok(&c, &["commit", "-m", "add oauth"], &[]);
    let denied = fail(&c, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert!(denied.contains("protected"), "{denied}");
    let published = ok(
        &c,
        &["push", "--bookmark", "agents/claude-code/oauth", "-r", "@-"],
        &[],
    );
    assert_eq!(
        published["pushed"][0]["bookmark"],
        "agents/claude-code/oauth"
    );

    // Ana pulls the agent's work and merges it into main herself.
    ok(&a, &["pull"], &[]);
    ok(&a, &["merge", "agents/claude-code/oauth@origin"], &[]);
    ok(&a, &["commit", "-m", "merge oauth"], &[]);
    ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    // Luis pushes on top of the old main: rejected instead of overwriting.
    fs::write(b.join("readme.md"), "hi\n").unwrap();
    ok(&b, &["commit", "-m", "readme"], &[]);
    let stale = fail(&b, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert!(stale.contains("pull"), "{stale}");

    // After pulling, main has diverged and is merged locally.
    let pulled = ok(&b, &["pull"], &[]);
    assert_eq!(pulled["diverged"][0], "main");
    ok(&b, &["merge", "main@origin"], &[]);
    assert!(b.join("oauth.rs").exists());

    // Presence: Luis edits app.rs while Ana is editing it on her machine.
    fs::write(a.join("app.rs"), "fn main() { ana(); }\n").unwrap();
    ok(&a, &["status"], &[]);
    fs::write(b.join("app.rs"), "fn main() { luis(); }\n").unwrap();
    let status = ok(&b, &["status"], &[]);
    let remote = status["warnings"]["remote"].as_array().unwrap();
    assert!(
        remote.iter().any(|w| w["path"] == "app.rs"),
        "expected a cross-machine warning: {status}"
    );
}

#[test]
fn live_stream_announces_pushes() {
    use std::io::{BufRead, BufReader};

    let env = start_server();
    let stream_url = env.url.replace(
        &format!("/{}", env.repo),
        &format!("/api/v1/repos/{}/stream", env.repo),
    );
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        let mut resp = agent.get(&stream_url).call().unwrap();
        let reader = BufReader::new(resp.body_mut().as_reader());
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some(kind) = line.strip_prefix("event: ")
                && tx.send(kind.to_owned()).is_err()
            {
                break;
            }
        }
    });
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
        "ready"
    );

    let a = clone_dir(&env, "ana", &env.human);
    fs::write(a.join("x.txt"), "x\n").unwrap();
    ok(&a, &["commit", "-m", "x"], &[]);
    ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
        "bookmark"
    );
}

#[test]
fn reviews_need_a_person_and_merge_on_the_server() {
    let env = start_server();
    let a = clone_dir(&env, "ana", &env.human);
    fs::write(a.join("app.rs"), "one\ntwo\nthree\n").unwrap();
    ok(&a, &["commit", "-m", "base"], &[]);
    ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    // The agent publishes a change and asks for review.
    let c = clone_dir(&env, "agent", &env.agent);
    ok(&c, &["pull"], &[]);
    ok(&c, &["new", "main"], &[]);
    fs::write(c.join("app.rs"), "ONE\ntwo\nthree\n").unwrap();
    ok(&c, &["commit", "-m", "uppercase one"], &[]);
    ok(
        &c,
        &["push", "--bookmark", "agents/claude-code/upper", "-r", "@-"],
        &[],
    );
    let review = ok(
        &c,
        &[
            "review",
            "open",
            "agents/claude-code/upper",
            "--title",
            "Uppercase the first line",
        ],
        &[],
    );
    let id = review["id"].as_i64().unwrap().to_string();

    // Meanwhile Ana changes another line, so a real merge commit is needed.
    fs::write(a.join("app.rs"), "one\ntwo\nTHREE\n").unwrap();
    ok(&a, &["commit", "-m", "uppercase three"], &[]);
    ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    // The agent cannot approve its own work into main.
    let denied = fail(&c, &["review", "merge", &id], &[]);
    assert!(
        denied.contains("protected") || denied.contains("may not"),
        "{denied}"
    );

    // Ana approves: the server merges both edits.
    let merged = ok(&a, &["review", "merge", &id], &[]);
    assert_eq!(merged["state"], "merged");
    ok(&a, &["pull"], &[]);
    ok(&a, &["new", "main"], &[]);
    assert_eq!(
        fs::read_to_string(a.join("app.rs")).unwrap(),
        "ONE\ntwo\nTHREE\n"
    );
    let shown = ok(&a, &["show", "main"], &[]);
    assert_eq!(shown["parents"].as_array().unwrap().len(), 2);
    assert_eq!(shown["author"]["name"], "ana");

    // A conflicting review is refused with an explanation.
    fs::write(c.join("app.rs"), "ONE\ntwo\ntres\n").unwrap();
    ok(&c, &["commit", "-m", "spanish"], &[]);
    ok(
        &c,
        &["push", "--bookmark", "agents/claude-code/upper", "-r", "@-"],
        &[],
    );
    let second = ok(
        &c,
        &[
            "review",
            "open",
            "agents/claude-code/upper",
            "--title",
            "Spanish",
        ],
        &[],
    );
    let id2 = second["id"].as_i64().unwrap().to_string();
    let conflict = fail(&a, &["review", "merge", &id2], &[]);
    assert!(conflict.contains("conflict"), "{conflict}");
    ok(&a, &["review", "close", &id2], &[]);

    // Cleaning up the agent's bookmark.
    ok(&a, &["push", "--delete", "agents/claude-code/upper"], &[]);
    let listed = ok(&a, &["review", "list", "--all"], &[]);
    assert_eq!(listed["reviews"].as_array().unwrap().len(), 2);
}
