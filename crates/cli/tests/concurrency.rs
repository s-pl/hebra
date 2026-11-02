//! Many processes on one repository at the same time, including processes
//! killed half-way. The promise: nobody fails, nobody waits for someone
//! else's work, and nothing is ever lost.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

fn hb_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hb"));
    command
        .current_dir(dir)
        .args(args)
        .env_remove("CLAUDECODE")
        .env_remove("HB_ON_BEHALF_OF")
        .env("HB_ACTOR", "tester")
        .env("HB_ACTOR_KIND", "human");
    command
}

fn run(dir: &Path, args: &[&str]) -> Output {
    hb_command(dir, args).output().unwrap()
}

#[track_caller]
fn hb(dir: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = run(dir, &full);
    assert!(
        out.status.success(),
        "hb {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    hb(&root, &["init"]);
    hb(&root, &["config", "set", "user.name", "tester"]);
    fs::write(root.join("base.txt"), "base\n").unwrap();
    hb(&root, &["commit", "-m", "base"]);
    (dir, root)
}

fn descriptions(root: &Path) -> Vec<String> {
    hb(root, &["log", "-n", "100000", "--all"])["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["description"].as_str().unwrap_or("").to_owned())
        .collect()
}

#[track_caller]
fn assert_healthy(root: &Path) {
    let out = run(root, &["fsck"]);
    assert!(
        out.status.success(),
        "fsck: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn many_agents_commit_at_once_and_nothing_is_lost() {
    const AGENTS: usize = 24;
    const COMMITS: usize = 3;
    let (_dir, root) = repo();
    let mut paths = Vec::new();
    for i in 0..AGENTS {
        let out = hb(
            &root,
            &["start", &format!("agent {i}"), "--agent", &format!("a{i}")],
        );
        paths.push(PathBuf::from(out["path"].as_str().unwrap()));
    }
    let failures = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            let failures = Arc::clone(&failures);
            std::thread::spawn(move || {
                for k in 0..COMMITS {
                    fs::write(path.join(format!("file-{i}-{k}.txt")), "x\n").unwrap();
                    let status = run(&path, &["status"]);
                    let commit = run(&path, &["commit", "-m", &format!("work {i} {k}")]);
                    for out in [status, commit] {
                        if !out.status.success() {
                            eprintln!("{}", String::from_utf8_lossy(&out.stderr));
                            failures.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(failures.load(Ordering::Relaxed), 0);
    let log = descriptions(&root);
    for i in 0..AGENTS {
        for k in 0..COMMITS {
            assert!(
                log.contains(&format!("work {i} {k}")),
                "work {i} {k} is gone"
            );
        }
    }
    assert_healthy(&root);
}

#[test]
fn commands_in_the_same_workspace_take_turns() {
    const PROCESSES: usize = 12;
    let (_dir, root) = repo();
    let children: Vec<(usize, Child)> = (0..PROCESSES)
        .map(|i| {
            fs::write(root.join(format!("same-{i}.txt")), "x\n").unwrap();
            let child = hb_command(&root, &["commit", "-m", &format!("same {i}")])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            (i, child)
        })
        .collect();
    for (i, child) in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{i}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let log = descriptions(&root);
    for i in 0..PROCESSES {
        assert!(log.contains(&format!("same {i}")), "same {i} is gone");
    }
    // Every file made it into a change.
    let files = hb(&root, &["ls-files"]);
    assert_eq!(files["files"].as_array().unwrap().len(), PROCESSES + 1);
    assert_healthy(&root);
}

#[test]
fn readers_do_not_wait_for_a_long_command() {
    let (_dir, root) = repo();
    // A big import in one workspace...
    let other = hb(&root, &["start", "reader", "--agent", "r"]);
    let reader = PathBuf::from(other["path"].as_str().unwrap());
    for i in 0..400 {
        fs::write(root.join(format!("bulk-{i}.txt")), format!("{i}\n")).unwrap();
    }
    let mut busy = hb_command(&root, &["commit", "-m", "bulk"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // ...does not stop another workspace from reading or writing.
    fs::write(reader.join("mine.txt"), "x\n").unwrap();
    hb(&reader, &["status"]);
    hb(&reader, &["commit", "-m", "while the other runs"]);
    assert!(busy.wait().unwrap().success());
    let log = descriptions(&root);
    assert!(log.contains(&"bulk".to_owned()));
    assert!(log.contains(&"while the other runs".to_owned()));
    assert_healthy(&root);
}

#[test]
fn a_killed_command_leaves_a_working_repository() {
    let (_dir, root) = repo();
    for round in 0..12 {
        for i in 0..40 {
            fs::write(root.join(format!("k-{round}-{i}.txt")), "data\n").unwrap();
        }
        let mut child = hb_command(&root, &["commit", "-m", &format!("round {round}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // At a different moment each time.
        std::thread::sleep(std::time::Duration::from_millis(2 + round * 3));
        let _ = child.kill();
        let _ = child.wait();
    }
    // The lock died with the process; everything still works.
    fs::write(root.join("after.txt"), "x\n").unwrap();
    hb(&root, &["commit", "-m", "after the crashes"]);
    assert_healthy(&root);
    // Every file is there, in some change or in the working copy.
    let tree = hb(&root, &["ls-tree", "-r", "--name-only", "@-"]);
    assert_eq!(tree["entries"].as_array().unwrap().len(), 12 * 40 + 2);
}

#[test]
fn a_workspace_whose_change_was_moved_catches_up_keeping_edits() {
    let (_dir, root) = repo();
    hb(&root, &["branch", "main"]);
    let out = hb(
        &root,
        &["start", "feature", "--agent", "a", "--from", "main"],
    );
    let other = PathBuf::from(out["path"].as_str().unwrap());

    // The agent has work in its directory; meanwhile its base moves.
    fs::write(other.join("agent.txt"), "in progress\n").unwrap();
    hb(&other, &["status"]);
    fs::write(root.join("from-main.txt"), "new on main\n").unwrap();
    hb(&root, &["commit", "-m", "main moved"]);
    hb(&root, &["rebase", "-s", "feature@", "-d", "@-"]);

    // More edits, made while it was behind.
    fs::write(other.join("agent.txt"), "still in progress\n").unwrap();
    let status = hb(&other, &["status"]);
    assert_eq!(status["changes"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(other.join("from-main.txt")).unwrap(),
        "new on main\n",
        "the rebased files arrived"
    );
    assert_eq!(
        fs::read_to_string(other.join("agent.txt")).unwrap(),
        "still in progress\n",
        "the edits survived"
    );
    assert_healthy(&root);
}

#[test]
fn interrupted_updates_finish_on_the_next_command() {
    let (_dir, root) = repo();
    fs::write(root.join("two.txt"), "two\n").unwrap();
    hb(&root, &["commit", "-m", "two"]);
    // The view moved but the files did not, as if the process died right
    // after publishing.
    let cache = root.join(".hebra/wc-cache.json");
    let saved = fs::read(&cache).unwrap();
    hb(&root, &["new", "@--"]);
    assert!(!root.join("two.txt").exists());
    fs::write(root.join("two.txt"), "two\n").unwrap();
    fs::write(&cache, saved).unwrap();
    let status = hb(&root, &["status"]);
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);
    assert!(!root.join("two.txt").exists(), "the checkout finished");
}
