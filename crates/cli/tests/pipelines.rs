//! Pipelines end to end: steps, a matrix, checks that wait for others,
//! secrets masked in logs, artifacts, path filters, machines registered
//! with a one-time token, and results reused when inputs did not change.

mod common;

use std::fs;

use serde_json::{Value, json};

fn commit_and_push(env: &common::Env, dir: &std::path::Path, file: &str, text: &str) {
    if let Some(parent) = dir.join(file).parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(dir.join(file), text).unwrap();
    env.ok(dir, &["commit", "-m", &format!("edit {file}")], &[]);
    env.ok(dir, &["push", "--bookmark", "main", "-r", "@-"], &[]);
}

/// The runs of `main`'s commit, once the push has queued all five checks
/// (the server queues them just after answering the push).
fn checks(env: &common::Env, dir: &std::path::Path) -> Vec<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let runs = env.ok(dir, &["checks", "list", "main"], &[])["checks"]
            .as_array()
            .unwrap()
            .clone();
        if runs.len() >= 5 || std::time::Instant::now() > deadline {
            return runs;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn by_name<'a>(runs: &'a [Value], name: &str) -> &'a Value {
    runs.iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no run `{name}` in {runs:?}"))
}

fn log_of(env: &common::Env, dir: &std::path::Path, id: &str) -> String {
    env.ok(dir, &["checks", "log", id], &[])["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["text"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn pipelines_run_steps_in_order_with_secrets_artifacts_and_reuse() {
    let env = common::start_server();
    let ana = env.clone_dir("ana", &env.human);
    let (show_secret, package) = if cfg!(windows) {
        (
            "echo key=%DEPLOY_KEY%",
            "mkdir out && echo built> out\\pkg.txt",
        )
    } else {
        (
            "echo key=$DEPLOY_KEY",
            "mkdir -p out && echo built > out/pkg.txt",
        )
    };
    fs::create_dir_all(ana.join(".hb")).unwrap();
    fs::write(
        ana.join(".hb/checks.toml"),
        format!(
            r#"
[[check]]
name = "test"
on = ["push:main"]
matrix = {{ part = ["a", "b"] }}
secrets = ["DEPLOY_KEY"]
steps = [
  {{ name = "hello", run = "echo part ${{{{ matrix.part }}}}" }},
  {{ name = "secret", run = "{show_secret}" }},
  {{ name = "only on failure", run = "echo never", if = "failure" }},
]

[[check]]
name = "package"
on = ["push:main"]
needs = ["test"]
inputs = ["src/**"]
run = '{package}'
artifacts = ["out/pkg.txt"]

[[check]]
name = "docs"
on = ["push:main"]
paths = ["docs/**"]
run = "echo docs"

[[check]]
name = "deploy"
on = ["push:main", "push:feature"]
needs = ["package"]
environment = "production"
secrets = ["DEPLOY_KEY"]
run = "{show_secret}"
"#
        ),
    )
    .unwrap();
    let (status, _) = env.api(
        "PUT",
        &format!("/repos/{}/secrets/DEPLOY_KEY", env.repo),
        &env.human,
        Some(json!({ "value": "s3cr3t-value" })),
    );
    assert_eq!(status, 200);
    // Production: one approval, only from main, its own key.
    let (status, _) = env.api(
        "PUT",
        &format!("/repos/{}/environments/production", env.repo),
        &env.human,
        Some(
            json!({ "required_approvals": 1, "branches": ["main"], "url": "https://acme.example" }),
        ),
    );
    assert_eq!(status, 200);
    let (status, _) = env.api(
        "PUT",
        &format!(
            "/repos/{}/environments/production/secrets/DEPLOY_KEY",
            env.repo
        ),
        &env.human,
        Some(json!({ "value": "prod-only-key" })),
    );
    assert_eq!(status, 200);
    commit_and_push(&env, &ana, "src/lib.rs", "pub fn f() {}\n");

    // A machine registers with a one-time token, no person signed in on it.
    let home = env.root.join("home");
    fs::create_dir_all(&home).unwrap();
    let creds =
        json!({ "servers": { env.server.clone(): { "mandate": env.human, "handle": env.ana() } } });
    fs::create_dir_all(&env.config_dir).unwrap();
    fs::write(env.config_dir.join("credentials.json"), creds.to_string()).unwrap();
    let token = env.ok(
        &home,
        &[
            "runner",
            "token",
            "--owner",
            env.ana(),
            "--server",
            &env.server,
        ],
        &[],
    );
    let token = token["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("hbr_"));
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
            "vps-1",
        ],
        &[],
    );
    // Used up.
    env.fail(
        &home,
        &[
            "runner",
            "register",
            "--server",
            &env.server,
            "--token",
            &token,
            "--name",
            "vps-2",
        ],
        &[],
    );

    // Nothing changed under docs/: skipped without a runner.
    let runs = checks(&env, &ana);
    assert_eq!(by_name(&runs, "docs")["state"], "skipped");
    assert_eq!(by_name(&runs, "package")["state"], "queued");

    // The matrix runs first; `package` waits for both.
    for _ in 0..2 {
        env.ok(&home, &["runner", "start", "vps-1", "--once"], &[]);
    }
    let runs = checks(&env, &ana);
    let a = by_name(&runs, "test (a)");
    assert_eq!(a["state"], "success", "{runs:?}");
    assert_eq!(by_name(&runs, "test (b)")["state"], "success");
    assert_eq!(by_name(&runs, "package")["state"], "queued");
    let log = log_of(&env, &ana, a["id"].as_str().unwrap());
    assert!(log.contains("part a"), "{log}");
    assert!(
        log.contains("key=***") && !log.contains("s3cr3t-value"),
        "{log}"
    );
    assert!(log.contains("only on failure (skipped)"), "{log}");

    env.ok(&home, &["runner", "start", "vps-1", "--once"], &[]);
    let runs = checks(&env, &ana);
    let pkg = by_name(&runs, "package");
    assert_eq!(pkg["state"], "success", "{runs:?}");
    let (status, artifacts) = env.api(
        "GET",
        &format!(
            "/repos/{}/checks/{}/artifacts",
            env.repo,
            pkg["id"].as_str().unwrap()
        ),
        &env.human,
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(artifacts["artifacts"][0]["name"], "pkg.txt", "{artifacts}");

    // The deployment waits for a person.
    let runs = checks(&env, &ana);
    let deploy = by_name(&runs, "deploy").clone();
    assert_eq!(deploy["state"], "waiting", "{runs:?}");
    env.ok(&home, &["runner", "start", "vps-1", "--once"], &[]);
    assert_eq!(by_name(&checks(&env, &ana), "deploy")["state"], "waiting");
    let (status, approved) = env.api(
        "POST",
        &format!(
            "/repos/{}/checks/{}/approve",
            env.repo,
            deploy["id"].as_str().unwrap()
        ),
        &env.human,
        None,
    );
    assert_eq!(status, 200, "{approved}");
    assert_eq!(approved["released"], true);
    env.ok(&home, &["runner", "start", "vps-1", "--once"], &[]);
    let runs = checks(&env, &ana);
    assert_eq!(by_name(&runs, "deploy")["state"], "success", "{runs:?}");
    let log = log_of(&env, &ana, deploy["id"].as_str().unwrap());
    assert!(
        log.contains("key=***") && !log.contains("prod-only-key"),
        "{log}"
    );
    let (_, history) = env.api(
        "GET",
        &format!("/repos/{}/environments/production/deployments", env.repo),
        &env.human,
        None,
    );
    assert_eq!(history["deployments"][0]["state"], "success", "{history}");
    assert_eq!(history["deployments"][0]["approved_by"][0], env.ana());

    // A change outside `src/`: the tests run again, `package` is reused.
    commit_and_push(&env, &ana, "README.md", "# acme\n");
    let runs = checks(&env, &ana);
    let pkg = by_name(&runs, "package");
    assert_eq!(pkg["state"], "success", "{runs:?}");
    assert!(
        pkg["summary"].as_str().unwrap().starts_with("reused"),
        "{pkg}"
    );
    assert_eq!(by_name(&runs, "test (a)")["state"], "queued");

    // Heddle's view of it.
    let (status, insights) = env.api(
        "GET",
        &format!("/repos/{}/insights", env.repo),
        &env.human,
        None,
    );
    assert_eq!(status, 200);
    let test = insights["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "test")
        .unwrap();
    assert_eq!(test["executed"], 2, "{insights}");
    let package = insights["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "package")
        .unwrap();
    assert_eq!(package["reused"], 1, "{insights}");
}
