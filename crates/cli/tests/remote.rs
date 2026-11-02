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

#[test]
fn partial_clones_read_contents_as_they_need_them() {
    let env = common::start_server();
    let a = env.clone_dir("a", &env.human);
    for (i, text) in ["one\n", "two\n", "three\n"].iter().enumerate() {
        std::fs::write(a.join("f.txt"), text).unwrap();
        std::fs::write(
            a.join(format!("old-{i}.txt")),
            format!(
                "file {i}
"
            ),
        )
        .unwrap();
        env.ok(&a, &["commit", "-m", &format!("v{i}")], &[]);
    }
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);

    env.ok(
        &env.root,
        &[
            "clone",
            "--filter=blob:none",
            "--token",
            &env.human,
            &env.url,
            "partial",
        ],
        &[],
    );
    let dir = env.root.join("partial");
    // The tip is there...
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).unwrap(),
        "three\n"
    );
    // ...history is known without its contents...
    let log = env.ok(&dir, &["log", "-n", "10", "main"], &[]);
    assert_eq!(log["commits"].as_array().unwrap().len(), 4);
    let before = env.ok(&dir, &["stats"], &[]);
    // ...and old contents arrive when read.
    let old = env.hb(&dir, &["cat", "main~2:f.txt"], &[]);
    assert_eq!(String::from_utf8_lossy(&old.stdout), "one\n");
    let after = env.ok(&dir, &["stats"], &[]);
    assert!(after["loose_objects"].as_u64() > before["loose_objects"].as_u64());
    env.ok(&dir, &["fsck"], &[]);

    // Working and pushing from a partial clone is ordinary.
    std::fs::write(dir.join("new.txt"), "from partial\n").unwrap();
    env.ok(&dir, &["commit", "-m", "partial work"], &[]);
    env.ok(&dir, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    env.ok(&a, &["pull"], &[]);
    env.ok(&a, &["new", "main"], &[]);
    assert_eq!(
        std::fs::read_to_string(a.join("new.txt")).unwrap(),
        "from partial\n"
    );
}

#[test]
fn tags_travel_through_a_server_with_their_messages() {
    let env = common::start_server();
    let a = env.clone_dir("a", &env.human);
    std::fs::write(a.join("f.txt"), "one\n").unwrap();
    env.ok(&a, &["commit", "-m", "first"], &[]);
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    env.ok(&a, &["tag", "light"], &[]);
    env.ok(&a, &["tag", "-a", "v1.0", "-m", "the first release"], &[]);
    env.ok(&a, &["push", "--tags"], &[]);

    let b = env.clone_dir("b", &env.human);
    env.ok(&b, &["pull"], &[]);
    let tags = env.ok(&b, &["tag", "-l"], &[]);
    let names: Vec<&str> = tags["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["light", "v1.0"]);
    let v1 = &tags["tags"][1]["tag"];
    assert_eq!(v1["message"], "the first release");
    let main = env.ok(&b, &["rev-parse", "main"], &[]);
    assert_eq!(v1["target"], main["revisions"][0]);
    assert_eq!(tags["tags"][0]["tag"]["target"], main["revisions"][0]);
}

#[test]
fn the_server_answers_blame_compare_and_search() {
    let env = common::start_server();
    let a = env.clone_dir("a", &env.human);
    std::fs::write(a.join("pay.rs"), "fn charge() {}\nfn refund() {}\n").unwrap();
    env.ok(&a, &["commit", "-m", "payments"], &[]);
    env.ok(&a, &["push", "--bookmark", "main", "-r", "@-"], &[]);
    std::fs::write(a.join("pay.rs"), "fn charge() {}\nfn refund_all() {}\n").unwrap();
    env.ok(&a, &["commit", "-m", "refund everything"], &[]);
    env.ok(&a, &["push", "--bookmark", "topic", "-r", "@-"], &[]);

    let repo = format!("/repos/{}", env.repo);
    let (status, blame) = env.api(
        "GET",
        &format!("{repo}/blame/topic?path=pay.rs"),
        &env.human,
        None,
    );
    assert_eq!(status, 200, "{blame}");
    let line = &blame["lines"][1];
    assert_eq!(line["text"], "fn refund_all() {}");
    let commit = line["commit"].as_str().unwrap();
    assert_eq!(blame["commits"][commit]["description"], "refund everything");

    let (status, compare) = env.api(
        "GET",
        &format!("{repo}/compare?base=main&head=topic"),
        &env.human,
        None,
    );
    assert_eq!(status, 200, "{compare}");
    assert_eq!(compare["commits"].as_array().unwrap().len(), 1);
    assert_eq!(compare["files"][0]["path"], "pay.rs");

    let (status, search) = env.api(
        "GET",
        &format!("{repo}/search?q=REFUND&rev=main"),
        &env.human,
        None,
    );
    assert_eq!(status, 200, "{search}");
    assert_eq!(search["matches"][0]["line"], 2);
    assert_eq!(search["matches"][0]["path"], "pay.rs");
    let (status, _) = env.api("GET", &format!("{repo}/search?q=x"), &env.eve, None);
    assert_eq!(
        status, 404,
        "a private repository does not even exist for strangers"
    );
}
