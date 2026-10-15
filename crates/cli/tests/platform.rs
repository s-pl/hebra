//! End-to-end tests of the platform: signing in, mandates, organizations,
//! privacy, tasks, checks with a real runner, policy, contributions, and
//! billing limits.

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use common::Env;
use serde_json::json;

fn commit_and_push(
    env: &Env,
    dir: &std::path::Path,
    file: &str,
    text: &str,
    bookmark: &str,
    extra: &[(&str, &str)],
) {
    fs::write(dir.join(file), text).unwrap();
    env.ok(dir, &["commit", "-m", &format!("edit {file}")], extra);
    env.ok(dir, &["push", "--bookmark", bookmark, "-r", "@-"], extra);
}

#[test]
fn login_with_the_device_flow_then_delegate_to_an_agent() {
    let env = common::start_server();
    let home = env.root.join("home");
    fs::create_dir_all(&home).unwrap();

    // `hb login` prints the code (JSON on stderr) and waits for approval.
    let mut child = Command::new(env!("CARGO_BIN_EXE_hb"))
        .current_dir(&home)
        .args(["--json", "login", &env.server])
        .env("HB_CONFIG_DIR", &env.config_dir)
        .env_remove("HB_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let start: serde_json::Value = serde_json::from_str(&line).unwrap();
    let code = start["user_code"].as_str().unwrap();

    // Ana approves it in the browser (her web session).
    let (status, shown) = env.api("GET", &format!("/auth/device/{code}"), &env.session, None);
    assert_eq!(status, 200, "{shown}");
    let (status, _) = env.api(
        "POST",
        &format!("/auth/device/{code}"),
        &env.session,
        Some(json!({ "approve": true })),
    );
    assert_eq!(status, 200);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The stored mandate works.
    let me = env.ok(&home, &["whoami", &env.server], &[]);
    assert_eq!(me["account"]["handle"], env.ana());

    // A narrowed mandate for an agent: its own bookmarks, this repository.
    let granted = env.ok(
        &home,
        &[
            "mandate",
            "grant",
            "--agent",
            "codex",
            "--repo",
            &env.repo,
            "--ttl",
            "1h",
            "--server",
            &env.server,
        ],
        &[],
    );
    let agent_mandate = granted["mandate"].as_str().unwrap().to_owned();
    let work = env.clone_dir("codex", &agent_mandate);
    env.ok(&work, &["pull"], &[]);
    fs::write(work.join("a.txt"), "a\n").unwrap();
    env.ok(&work, &["commit", "-m", "a"], &[]);
    env.ok(
        &work,
        &["push", "--bookmark", "agents/codex/a", "-r", "@-"],
        &[],
    );
    let denied = env.fail(&work, &["push", "--bookmark", "feature/x", "-r", "@-"], &[]);
    assert!(denied.contains("mandate"), "{denied}");

    // The agent is recorded as such, on behalf of Ana.
    let (_, events) = env.api(
        "GET",
        &format!("/repos/{}/events", env.repo),
        &env.human,
        None,
    );
    let actor = events["events"][0]["actor"].as_str().unwrap();
    assert!(actor.starts_with("codex (agent for "), "{actor}");

    // Signing out revokes the stored mandate, and everything derived from it.
    env.ok(&home, &["logout", &env.server], &[]);
    let refused = env.fail(
        &work,
        &["push", "--bookmark", "agents/codex/b", "-r", "@-"],
        &[],
    );
    assert!(
        refused.contains("401") || refused.contains("revoked"),
        "{refused}"
    );
}

#[test]
fn private_repositories_and_organizations() {
    let env = common::start_server();
    // Strangers and anonymous callers cannot even see a private repository.
    let (status, _) = env.api("GET", &format!("/repos/{}", env.repo), &env.bob, None);
    assert_eq!(status, 404);
    let (status, _) = env.api("GET", &format!("/repos/{}", env.repo), "", None);
    assert_eq!(status, 404);
    let (_, listed) = env.api("GET", "/repos", &env.bob, None);
    assert!(
        listed["repos"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["full_name"] != env.repo.as_str())
    );

    // An organization: Ana owns it, Bob joins as a member and gets read on
    // the organization's private repositories.
    let org = format!("org-{}", &env.ana()[4..]);
    let (status, body) = env.api(
        "POST",
        "/orgs",
        &env.human,
        Some(json!({ "handle": org, "name": "Org" })),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = env.api(
        "POST",
        "/repos",
        &env.human,
        Some(json!({ "owner": org, "name": "secret" })),
    );
    assert_eq!(status, 200, "{body}");
    let secret = format!("{org}/secret");
    let (status, _) = env.api("GET", &format!("/repos/{secret}"), &env.bob, None);
    assert_eq!(status, 404);
    let (status, _) = env.api(
        "PUT",
        &format!("/accounts/{org}/members/{}", env.bob_handle),
        &env.human,
        Some(json!({ "role": "member" })),
    );
    assert_eq!(status, 200);
    let (status, info) = env.api("GET", &format!("/repos/{secret}"), &env.bob, None);
    assert_eq!(status, 200, "{info}");
    assert_eq!(info["role"], "read");
    // Members cannot manage the organization; the last owner cannot leave.
    let (status, _) = env.api(
        "PUT",
        &format!("/accounts/{org}/members/{}", env.eve_handle),
        &env.bob,
        Some(json!({ "role": "member" })),
    );
    assert_eq!(status, 403);
    let (status, _) = env.api(
        "DELETE",
        &format!("/accounts/{org}/members/{}", env.ana()),
        &env.human,
        None,
    );
    assert_eq!(status, 400);

    // A team with write access lets Bob push.
    let (status, _) = env.api(
        "POST",
        &format!("/accounts/{org}/teams"),
        &env.human,
        Some(json!({ "handle": "devs" })),
    );
    assert_eq!(status, 200);
    env.api(
        "PUT",
        &format!("/accounts/{org}/teams/devs/members/{}", env.bob_handle),
        &env.human,
        None,
    );
    let (status, _) = env.api(
        "PUT",
        &format!("/repos/{secret}/access/team:devs"),
        &env.human,
        Some(json!({ "role": "write" })),
    );
    assert_eq!(status, 200);
    let (_, info) = env.api("GET", &format!("/repos/{secret}"), &env.bob, None);
    assert_eq!(info["role"], "write");
}

#[test]
fn tasks_are_claimed_with_leases_and_closed_by_merges() {
    let env = common::start_server();
    let ana = env.clone_dir("ana", &env.human);
    commit_and_push(&env, &ana, "app.rs", "fn main() {}\n", "main", &[]);

    let first = env.ok(
        &ana,
        &[
            "task",
            "create",
            "Add login",
            "--accept",
            "a test covers it",
        ],
        &[],
    );
    let n = first["id"].as_i64().unwrap().to_string();
    let second = env.ok(
        &ana,
        &["task", "create", "Document login", "--after", &n],
        &[],
    );
    let m = second["id"].as_i64().unwrap().to_string();
    // The second waits for the first.
    let ready = env.ok(&ana, &["task", "list", "--state", "ready"], &[]);
    assert_eq!(ready["tasks"].as_array().unwrap().len(), 1);
    env.fail(&ana, &["task", "claim", &m], &[]);

    // The agent claims the first; nobody else can meanwhile.
    let agent = env.clone_dir("agent", &env.agent);
    env.ok(&agent, &["pull"], &[]);
    env.ok(&agent, &["new", "main"], &[]);
    let claimed = env.ok(&agent, &["task", "claim", &n, "--for", "5"], &[]);
    assert_eq!(claimed["state"], "claimed");
    let busy = env.fail(&ana, &["task", "claim", &n], &[]);
    assert!(busy.contains("claimed"), "{busy}");

    // Its work names the task; merging the review closes it.
    fs::write(agent.join("login.rs"), "pub fn login() {}\n").unwrap();
    let task_env = [("HB_TASK", format!("#{n}"))];
    let task_env: Vec<(&str, &str)> = task_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    env.ok(&agent, &["commit", "-m", "login"], &task_env);
    env.ok(
        &agent,
        &["push", "--bookmark", "agents/claude-code/login", "-r", "@-"],
        &[],
    );
    let review = env.ok(
        &agent,
        &[
            "review",
            "open",
            "agents/claude-code/login",
            "--title",
            "Login",
        ],
        &[],
    );
    let id = review["id"].as_i64().unwrap().to_string();
    env.ok(&ana, &["review", "merge", &id], &[]);
    let done = env.ok(&ana, &["task", "show", &n], &[]);
    assert_eq!(done["state"], "done");
    let ready = env.ok(&ana, &["task", "list", "--state", "ready"], &[]);
    assert_eq!(ready["tasks"][0]["id"].as_i64().unwrap().to_string(), m);
}

#[test]
fn checks_run_on_a_registered_runner_and_gate_merges() {
    let env = common::start_server();
    let ana = env.clone_dir("ana", &env.human);
    fs::create_dir_all(ana.join(".hb")).unwrap();
    fs::write(
        ana.join(".hb/checks.toml"),
        "[[check]]\nname = \"build\"\nrun = \"echo building\"\non = [\"review\", \"push:main\"]\n\n\
         [[check]]\nname = \"marker\"\nrun = \"exit 3\"\non = [\"review\"]\n",
    )
    .unwrap();
    fs::write(
        ana.join(".hb/policy.toml"),
        "[bookmarks.main]\nmerge = \"review\"\nrequired_approvals = 1\nrequired_checks = [\"build\"]\n",
    )
    .unwrap();
    commit_and_push(&env, &ana, "app.rs", "fn main() {}\n", "main", &[]);

    // Bob may write; he approves reviews.
    env.api(
        "PUT",
        &format!("/repos/{}/access/{}", env.repo, env.bob_handle),
        &env.human,
        Some(json!({ "role": "write" })),
    );

    // Register a runner for Ana and let it run the push checks.
    let home = env.root.join("home");
    fs::create_dir_all(&home).unwrap();
    let creds =
        json!({ "servers": { env.server.clone(): { "mandate": env.human, "handle": env.ana() } } });
    fs::create_dir_all(&env.config_dir).unwrap();
    fs::write(env.config_dir.join("credentials.json"), creds.to_string()).unwrap();
    env.ok(
        &home,
        &[
            "runner",
            "register",
            "--owner",
            env.ana(),
            "--name",
            "box",
            "--server",
            &env.server,
        ],
        &[],
    );
    env.ok(&home, &["runner", "start", "box", "--once"], &[]);
    let checks = env.ok(&ana, &["checks", "list", "main"], &[]);
    assert_eq!(checks["checks"][0]["name"], "build");
    assert_eq!(checks["checks"][0]["state"], "success");
    let log = env.ok(
        &ana,
        &["checks", "log", checks["checks"][0]["id"].as_str().unwrap()],
        &[],
    );
    let text: String = log["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["text"].as_str().unwrap())
        .collect();
    assert!(text.contains("building"), "{text}");

    // Direct pushes to main are refused by policy now.
    fs::write(ana.join("app.rs"), "fn main() { 1; }\n").unwrap();
    env.ok(&ana, &["commit", "-m", "direct"], &[]);
    let refused = env.fail(&ana, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert!(
        refused.contains("only changes through reviews"),
        "{refused}"
    );

    // A review: checks run, the failing one is not required.
    env.ok(&ana, &["push", "--bookmark", "feature", "-r", "@-"], &[]);
    let review = env.ok(
        &ana,
        &["review", "open", "feature", "--title", "Feature"],
        &[],
    );
    let id = review["id"].as_i64().unwrap().to_string();
    let not_ready = env.fail(&ana, &["review", "merge", &id], &[]);
    assert!(not_ready.contains("not ready"), "{not_ready}");
    env.ok(&home, &["runner", "start", "box", "--once"], &[]);
    env.ok(&home, &["runner", "start", "box", "--once"], &[]);
    // Ana cannot approve her own review; Bob can.
    let own = env.fail(&ana, &["review", "approve", &id], &[]);
    assert!(own.contains("own review"), "{own}");
    let bob = env.clone_dir("bob", &env.bob);
    let approved = env.ok(&bob, &["review", "approve", &id], &[]);
    assert_eq!(approved["requirements"]["ready"], true, "{approved}");
    let failing: Vec<_> = approved["requirements"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["state"] == "failure")
        .collect();
    assert_eq!(failing.len(), 1);
    assert_eq!(failing[0]["name"], "marker");
    let merged = env.ok(&ana, &["review", "merge", &id], &[]);
    assert_eq!(merged["state"], "merged");
}

#[test]
fn contributions_without_forks() {
    let env = common::start_server();
    let ana = env.clone_dir("ana", &env.human);
    commit_and_push(&env, &ana, "app.rs", "fn main() {}\n", "main", &[]);
    env.api(
        "PATCH",
        &format!("/repos/{}", env.repo),
        &env.human,
        Some(json!({ "public": true, "open_contributions": true })),
    );

    // Eve has no role, but may propose from her own namespace.
    let eve = env.clone_dir("eve", &env.eve);
    env.ok(&eve, &["pull"], &[]);
    env.ok(&eve, &["new", "main"], &[]);
    fs::write(eve.join("fix.rs"), "fix\n").unwrap();
    env.ok(&eve, &["commit", "-m", "fix"], &[]);
    let refused = env.fail(&eve, &["push", "--bookmark", "fix", "-r", "@-"], &[]);
    assert!(refused.contains("403"), "{refused}");
    let own = format!("contrib/{}/fix", env.eve_handle);
    env.ok(&eve, &["push", "--bookmark", &own, "-r", "@-"], &[]);
    let review = env.ok(&eve, &["review", "open", &own, "--title", "A fix"], &[]);
    let id = review["id"].as_i64().unwrap().to_string();
    // She cannot merge it.
    let denied = env.fail(&eve, &["review", "merge", &id], &[]);
    assert!(denied.contains("403"), "{denied}");
    env.ok(&ana, &["review", "merge", &id], &[]);

    // A fork is instant and shares objects.
    let (status, fork) = env.api(
        "POST",
        &format!("/repos/{}/fork", env.repo),
        &env.eve,
        Some(json!({})),
    );
    assert_eq!(status, 200, "{fork}");
    let forked = format!("{}/acme", env.eve_handle);
    let (status, info) = env.api("GET", &format!("/repos/{forked}"), &env.eve, None);
    assert_eq!(status, 200);
    assert!(info["bookmarks"]["main"].is_string());
    let (status, file) = env.api(
        "GET",
        &format!("/repos/{forked}/tree/main?path=fix.rs"),
        &env.eve,
        None,
    );
    assert_eq!(status, 200, "{file}");
    assert_eq!(file["content"], "fix\n");
}

#[test]
fn agents_cannot_approve_or_mint_mandates() {
    let env = common::start_server();
    let ana = env.clone_dir("ana", &env.human);
    commit_and_push(&env, &ana, "app.rs", "fn main() {}\n", "main", &[]);
    env.ok(&ana, &["push", "--bookmark", "other", "-r", "@-"], &[]);
    let review = env.ok(&ana, &["review", "open", "other", "--title", "Other"], &[]);
    let id = review["id"].as_i64().unwrap();
    let (status, body) = env.api(
        "POST",
        &format!("/repos/{}/reviews/{id}/approve", env.repo),
        &env.agent,
        Some(json!({})),
    );
    assert_eq!(status, 403, "{body}");
    let (status, _) = env.api(
        "POST",
        "/me/mandates",
        &env.agent,
        Some(json!({ "label": "more" })),
    );
    assert_eq!(status, 403);
    let (status, _) = env.api(
        "POST",
        "/orgs",
        &env.agent,
        Some(json!({ "handle": "agent-org", "name": "x" })),
    );
    assert_eq!(status, 403);
}

#[test]
fn cloud_plans_limit_private_repositories() {
    let env = common::start(hebra_server::Edition::Cloud);
    let (status, plans) = env.api("GET", "/plans", "", None);
    assert_eq!(status, 200);
    assert!(
        plans["plan"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "free")
    );
    let (_, caps) = env.api("GET", "/capabilities", "", None);
    assert_eq!(caps["edition"], "cloud");
    assert_eq!(caps["features"]["merge_queue"], true);

    // Bob is on Free: 5 private repositories.
    for n in 0..5 {
        let (status, body) = env.api(
            "POST",
            "/repos",
            &env.bob,
            Some(json!({ "name": format!("p{n}") })),
        );
        assert_eq!(status, 200, "{body}");
    }
    let (status, body) = env.api("POST", "/repos", &env.bob, Some(json!({ "name": "p5" })));
    assert_eq!(status, 402, "{body}");
    assert!(body["error"]["message"].as_str().unwrap().contains("Free"));
    // Public repositories are unlimited.
    let (status, _) = env.api(
        "POST",
        "/repos",
        &env.bob,
        Some(json!({ "name": "open", "public": true })),
    );
    assert_eq!(status, 200);
    let (status, billing) = env.api(
        "GET",
        &format!("/accounts/{}/billing", env.bob_handle),
        &env.bob,
        None,
    );
    assert_eq!(status, 200, "{billing}");
    assert_eq!(billing["allowance"]["plan"]["id"], "free");
    assert_eq!(billing["credits_left"], 100);
    // Nobody else sees Bob's billing.
    let (status, _) = env.api(
        "GET",
        &format!("/accounts/{}/billing", env.bob_handle),
        &env.eve,
        None,
    );
    assert_eq!(status, 403);
}

/// A stand-in for the model provider: asks to read the contract, then
/// passes. Records what it was sent.
fn fake_model() -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let app = axum::Router::new().route(
                "/v1/messages",
                axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<serde_json::Value>| {
                    let record = Arc::clone(&record);
                    async move {
                        assert_eq!(headers["x-api-key"], "test-key");
                        let turns = body["messages"].as_array().unwrap().len();
                        record.lock().unwrap().push(body);
                        let content = if turns == 1 {
                            json!([{ "type": "tool_use", "id": "t1", "name": "run", "input": { "command": "echo tests-ok" } }])
                        } else {
                            json!([{ "type": "tool_use", "id": "t2", "name": "verdict",
                                     "input": { "verdict": "pass", "summary": "The contract holds.", "findings": [] } }])
                        };
                        axum::Json(json!({
                            "content": content,
                            "stop_reason": "tool_use",
                            "usage": { "input_tokens": 20000, "output_tokens": 1000 }
                        }))
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    (format!("http://{}", rx.recv().unwrap()), seen)
}

#[test]
fn agent_verification_by_contract() {
    let (model, seen) = fake_model();
    let env = common::start_with(
        hebra_server::Edition::Cloud,
        Some(hebra_server::ModelGateway {
            api_key: "test-key".into(),
            model: "claude-test".into(),
            base_url: model,
        }),
    );
    let ana = env.clone_dir("ana", &env.human);
    fs::create_dir_all(ana.join(".hb")).unwrap();
    fs::write(
        ana.join(".hb/contract.md"),
        "Every change runs `echo tests-ok`.\n",
    )
    .unwrap();
    fs::write(
        ana.join(".hb/checks.toml"),
        "[[check]]\nname = \"contract\"\nkind = \"agent\"\non = [\"push:main\"]\nmax_credits = 100\n",
    )
    .unwrap();
    commit_and_push(&env, &ana, "app.rs", "fn main() {}\n", "main", &[]);

    let home = env.root.join("home");
    fs::create_dir_all(&home).unwrap();
    let creds =
        json!({ "servers": { env.server.clone(): { "mandate": env.human, "handle": env.ana() } } });
    fs::create_dir_all(&env.config_dir).unwrap();
    fs::write(env.config_dir.join("credentials.json"), creds.to_string()).unwrap();
    env.ok(
        &home,
        &[
            "runner",
            "register",
            "--owner",
            env.ana(),
            "--name",
            "box",
            "--server",
            &env.server,
        ],
        &[],
    );
    env.ok(&home, &["runner", "start", "box", "--once"], &[]);

    let checks = env.ok(&ana, &["checks", "list", "main"], &[]);
    let run = &checks["checks"][0];
    assert_eq!(run["name"], "contract");
    assert_eq!(run["state"], "success", "{run}");
    // 20k input + 1k output tokens at $3/$15 per MTok = $0.075, +30% -> 10 credits per turn.
    assert_eq!(run["credits"], 20);
    // The model saw the contract and the command's output; it never chose
    // its own instructions or tools.
    let sent = seen.lock().unwrap();
    assert_eq!(sent[0]["model"], "claude-test");
    assert!(
        sent[0]["system"]
            .as_str()
            .unwrap()
            .contains("Hebra's verifier")
    );
    assert!(
        sent[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Every change runs")
    );
    assert!(
        sent[1]["messages"][2]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("tests-ok")
    );
    // The owner was charged.
    let (_, billing) = env.api(
        "GET",
        &format!("/accounts/{}/billing", env.ana()),
        &env.human,
        None,
    );
    assert_eq!(billing["credits_left"], 80);
}
