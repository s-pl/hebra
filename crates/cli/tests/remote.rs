//! End-to-end: a real hebra-server, two clones, people and agents.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use hebra_server::{Server, TokenInfo};
use serde_json::Value;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    url: String,
    human: String,
    agent: String,
}

fn start_server() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("server");
    let server = Server::open(&data).unwrap();
    server.create_repo("acme", &["main".to_owned()]).unwrap();
    let human = server
        .create_token(&TokenInfo {
            name: "ana".into(),
            kind: "human".into(),
            on_behalf_of: None,
            allow: vec!["*".into()],
            admin: true,
        })
        .unwrap();
    let agent = server
        .create_token(&TokenInfo {
            name: "claude-code".into(),
            kind: "agent".into(),
            on_behalf_of: Some("ana".into()),
            allow: vec!["agents/claude-code/*".into(), "main".into()],
            admin: false,
        })
        .unwrap();
    let server = Arc::new(server);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            hebra_server::serve(server, listener).await.unwrap();
        });
    });
    let addr = rx.recv().unwrap();
    let root = dir.path().to_path_buf();
    Env {
        _dir: dir,
        root,
        url: format!("http://{addr}/acme"),
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
