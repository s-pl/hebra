//! End-to-end: a real hebra-server, two clones, people and agents.

mod common;

use std::fs;

#[test]
fn push_pull_permissions_and_presence() {
    let env = common::start_server();

    // Ana publishes main.
    let a = env.clone_dir("ana", &env.human);
    fs::write(a.join("app.rs"), "fn main() {}\n").unwrap();
    env.ok(&a, &["commit", "-m", "first"], &[]);
    let pushed = env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert_eq!(pushed["pushed"][0]["bookmark"], "main");

    // A second machine pulls it and sees the files.
    let b = env.clone_dir("luis", &env.human);
    let pulled = env.ok(&b, &["pull"], &[]);
    assert_eq!(pulled["updated"][0], "main");
    env.ok(&b, &["new", "main"], &[]);
    assert_eq!(
        fs::read_to_string(b.join("app.rs")).unwrap(),
        "fn main() {}\n"
    );

    // An agent may publish to its own bookmarks but never move main.
    let c = env.clone_dir("agent", &env.agent);
    env.ok(&c, &["pull"], &[]);
    env.ok(&c, &["new", "main"], &[]);
    fs::write(c.join("oauth.rs"), "// oauth\n").unwrap();
    env.ok(&c, &["commit", "-m", "add oauth"], &[]);
    let denied = env.fail(&c, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert!(denied.contains("protected"), "{denied}");
    let published = env.ok(
        &c,
        &["push", "--bookmark", "agents/claude-code/oauth", "-r", "@-"],
        &[],
    );
    assert_eq!(
        published["pushed"][0]["bookmark"],
        "agents/claude-code/oauth"
    );

    // Ana pulls the agent's work and merges it into main herself.
    env.ok(&a, &["pull"], &[]);
    env.ok(&a, &["merge", "agents/claude-code/oauth@origin"], &[]);
    env.ok(&a, &["commit", "-m", "merge oauth"], &[]);
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    // Luis pushes on top of the old main: rejected instead of overwriting.
    fs::write(b.join("readme.md"), "hi\n").unwrap();
    env.ok(&b, &["commit", "-m", "readme"], &[]);
    let stale = env.fail(&b, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert!(stale.contains("pull"), "{stale}");

    // After pulling, main has diverged and is merged locally.
    let pulled = env.ok(&b, &["pull"], &[]);
    assert_eq!(pulled["diverged"][0], "main");
    env.ok(&b, &["merge", "main@origin"], &[]);
    assert!(b.join("oauth.rs").exists());

    // Presence: Luis edits app.rs while Ana is editing it on her machine.
    fs::write(a.join("app.rs"), "fn main() { ana(); }\n").unwrap();
    env.ok(&a, &["status"], &[]);
    fs::write(b.join("app.rs"), "fn main() { luis(); }\n").unwrap();
    let status = env.ok(&b, &["status"], &[]);
    let remote = status["warnings"]["remote"].as_array().unwrap();
    assert!(
        remote.iter().any(|w| w["path"] == "app.rs"),
        "expected a cross-machine warning: {status}"
    );
}

#[test]
fn live_stream_announces_pushes() {
    use std::io::{BufRead, BufReader};

    let env = common::start_server();
    let stream_url = env.url.replace(
        &format!("/{}", env.repo),
        &format!("/api/v1/repos/{}/stream", env.repo),
    );
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let token = env.human.clone();
    std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        let mut resp = agent
            .get(&stream_url)
            .header("Authorization", format!("Bearer {token}"))
            .call()
            .unwrap();
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

    let a = env.clone_dir("ana", &env.human);
    fs::write(a.join("x.txt"), "x\n").unwrap();
    env.ok(&a, &["commit", "-m", "x"], &[]);
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
        "bookmark"
    );
}

#[test]
fn reviews_need_a_person_and_merge_on_the_server() {
    let env = common::start_server();
    let a = env.clone_dir("ana", &env.human);
    fs::write(a.join("app.rs"), "one\ntwo\nthree\n").unwrap();
    env.ok(&a, &["commit", "-m", "base"], &[]);
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    // The agent publishes a change and asks for review.
    let c = env.clone_dir("agent", &env.agent);
    env.ok(&c, &["pull"], &[]);
    env.ok(&c, &["new", "main"], &[]);
    fs::write(c.join("app.rs"), "ONE\ntwo\nthree\n").unwrap();
    env.ok(&c, &["commit", "-m", "uppercase one"], &[]);
    env.ok(
        &c,
        &["push", "--bookmark", "agents/claude-code/upper", "-r", "@-"],
        &[],
    );
    let review = env.ok(
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
    env.ok(&a, &["commit", "-m", "uppercase three"], &[]);
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    // The agent cannot approve its own work into main.
    let denied = env.fail(&c, &["review", "merge", &id], &[]);
    assert!(
        denied.contains("protected") || denied.contains("may not"),
        "{denied}"
    );

    // Ana approves: the server merges both edits.
    let merged = env.ok(&a, &["review", "merge", &id], &[]);
    assert_eq!(merged["state"], "merged");
    env.ok(&a, &["pull"], &[]);
    env.ok(&a, &["new", "main"], &[]);
    assert_eq!(
        fs::read_to_string(a.join("app.rs")).unwrap(),
        "ONE\ntwo\nTHREE\n"
    );
    let shown = env.ok(&a, &["show", "main"], &[]);
    assert_eq!(shown["parents"].as_array().unwrap().len(), 2);
    assert_eq!(shown["author"]["name"], env.ana());

    // A conflicting review is refused with an explanation.
    fs::write(c.join("app.rs"), "ONE\ntwo\ntres\n").unwrap();
    env.ok(&c, &["commit", "-m", "spanish"], &[]);
    env.ok(
        &c,
        &["push", "--bookmark", "agents/claude-code/upper", "-r", "@-"],
        &[],
    );
    let second = env.ok(
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
    let conflict = env.fail(&a, &["review", "merge", &id2], &[]);
    assert!(conflict.contains("conflict"), "{conflict}");
    env.ok(&a, &["review", "close", &id2], &[]);

    // Cleaning up the agent's bookmark.
    env.ok(&a, &["push", "--delete", "agents/claude-code/upper"], &[]);
    let listed = env.ok(&a, &["review", "list", "--all"], &[]);
    assert_eq!(listed["reviews"].as_array().unwrap().len(), 2);
}
