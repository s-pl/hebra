//! End-to-end tests driving the `hb` binary.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn hb_command(cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hb"));
    cmd.current_dir(cwd)
        .env_remove("CLAUDECODE")
        .env_remove("HB_ACTOR")
        .env_remove("HB_ACTOR_KIND")
        .env_remove("HB_ON_BEHALF_OF")
        .env_remove("HB_TASK")
        .env_remove("HB_JSON")
        .env("USER", "tester")
        .env("USERNAME", "tester");
    cmd
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    hb_command(cwd)
        .args(args)
        .output()
        .expect("failed to run hb")
}

/// Runs `hb --json <args>`, asserts success and parses stdout.
fn json(cwd: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = run(cwd, &full);
    assert!(
        out.status.success(),
        "hb {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is not JSON")
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        Self { _dir: dir, root }
    }

    fn init(files: &[(&str, &str)]) -> Self {
        let repo = Self::new();
        for (path, content) in files {
            repo.write(path, content);
        }
        json(&repo.root, &["init"]);
        json(&repo.root, &["config", "set", "user.name", "ana"]);
        repo
    }

    fn write(&self, path: &str, content: &str) {
        write_in(&self.root, path, content);
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap()
    }
}

fn write_in(root: &Path, path: &str, content: &str) {
    let full = root.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

fn paths(changes: &Value) -> Vec<String> {
    changes
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn init_imports_existing_files_respecting_gitignore() {
    let repo = Repo::new();
    repo.write("src/main.rs", "fn main() {}\n");
    repo.write(".gitignore", "target/\n");
    repo.write("target/debug/app", "binary");
    let out = json(&repo.root, &["init"]);
    assert_eq!(out["imported_files"], 2);
    let status = json(&repo.root, &["status"]);
    assert_eq!(paths(&status["changes"]), [".gitignore", "src/main.rs"]);
}

#[test]
fn init_twice_fails_with_stable_error_code() {
    let repo = Repo::init(&[]);
    let out = run(&repo.root, &["--json", "init"]);
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["code"], "already_initialized");
}

#[test]
fn outside_a_repository_reports_not_a_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["--json", "status"]);
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["code"], "not_a_workspace");
}

#[test]
fn working_copy_is_recorded_without_staging() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    json(&repo.root, &["commit", "-m", "first"]);
    repo.write("a.txt", "2\n");
    repo.write("b.txt", "new\n");
    let status = json(&repo.root, &["status"]);
    assert_eq!(paths(&status["changes"]), ["a.txt", "b.txt"]);

    let out = json(&repo.root, &["commit", "-m", "second"]);
    assert_eq!(out["committed"]["description"], "second");
    let log = json(&repo.root, &["log"]);
    let descriptions: Vec<&str> = log["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["description"].as_str().unwrap())
        .collect();
    assert_eq!(descriptions, ["", "second", "first", ""]);
}

#[test]
fn line_endings_are_normalized() {
    let repo = Repo::init(&[]);
    repo.write("win.txt", "a\r\nb\r\n");
    json(&repo.root, &["commit", "-m", "crlf"]);
    let diff = json(&repo.root, &["diff", "-r", "@-"]);
    let patch = diff["files"][0]["patch"].as_str().unwrap();
    assert!(!patch.contains('\r'), "patch should be LF only: {patch:?}");
}

#[test]
fn change_ids_survive_rewrites() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    let before = json(&repo.root, &["status"])["working_copy"].clone();
    repo.write("a.txt", "2\n");
    let after = json(&repo.root, &["describe", "-m", "edited"]);
    assert_eq!(before["change_id"], after["change_id"]);
    assert_ne!(before["commit_id"], after["commit_id"]);
    let change = after["change_id"].as_str().unwrap();
    let shown = json(&repo.root, &["show", &change[..6]]);
    assert_eq!(shown["description"], "edited");
}

#[test]
fn agents_get_isolated_workspaces_and_see_each_other() {
    let repo = Repo::init(&[("main.rs", "fn main() {}\n")]);
    json(&repo.root, &["commit", "-m", "base"]);
    let a = json(&repo.root, &["start", "feature a", "--agent", "agent-a"]);
    let b = json(&repo.root, &["start", "feature b", "--agent", "agent-b"]);
    let a_path = PathBuf::from(a["path"].as_str().unwrap());
    let b_path = PathBuf::from(b["path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(a_path.join("main.rs")).unwrap(),
        "fn main() {}\n"
    );
    assert_eq!(a["actor"]["on_behalf_of"], "ana");

    write_in(&a_path, "main.rs", "fn main() { a(); }\n");
    write_in(&b_path, "main.rs", "fn main() { b(); }\n");
    json(&a_path, &["claim", "main.rs", "--note", "refactor"]);

    // Agent B sees both the claim and the overlapping edit, before any merge.
    let status = json(&b_path, &["status"]);
    assert_eq!(
        status["warnings"]["claimed_by_others"][0]["claim"]["workspace"],
        "feature-a"
    );
    assert_eq!(
        status["warnings"]["overlapping"][0]["workspace"],
        "feature-a"
    );

    // The main workspace was not touched.
    assert_eq!(repo.read("main.rs"), "fn main() {}\n");
    let ws = json(&repo.root, &["workspaces"]);
    assert_eq!(ws["workspaces"].as_array().unwrap().len(), 3);
}

#[test]
fn clean_merge_combines_agents_work() {
    let repo = Repo::init(&[("a.txt", "1\n2\n3\n")]);
    json(&repo.root, &["commit", "-m", "base"]);
    let a = PathBuf::from(
        json(&repo.root, &["start", "top", "--agent", "x"])["path"]
            .as_str()
            .unwrap(),
    );
    let b = PathBuf::from(
        json(&repo.root, &["start", "bottom", "--agent", "y"])["path"]
            .as_str()
            .unwrap(),
    );
    write_in(&a, "a.txt", "ONE\n2\n3\n");
    write_in(&b, "a.txt", "1\n2\nTHREE\n");
    write_in(&b, "b.txt", "added by y\n");
    json(&a, &["commit", "-m", "top"]);
    json(&b, &["commit", "-m", "bottom"]);

    let merged = json(&repo.root, &["merge", "top@-", "bottom@-"]);
    assert_eq!(merged["conflicts"].as_array().unwrap().len(), 0);
    assert_eq!(merged["parents"].as_array().unwrap().len(), 2);
    assert_eq!(repo.read("a.txt"), "ONE\n2\nTHREE\n");
    assert_eq!(repo.read("b.txt"), "added by y\n");
}

#[test]
fn conflicts_are_recorded_and_resolve_when_markers_go() {
    let repo = Repo::init(&[("a.txt", "base\n")]);
    json(&repo.root, &["commit", "-m", "base"]);
    let a = PathBuf::from(
        json(&repo.root, &["start", "a", "--agent", "x"])["path"]
            .as_str()
            .unwrap(),
    );
    let b = PathBuf::from(
        json(&repo.root, &["start", "b", "--agent", "y"])["path"]
            .as_str()
            .unwrap(),
    );
    write_in(&a, "a.txt", "from a\n");
    write_in(&b, "a.txt", "from b\n");
    json(&a, &["commit", "-m", "a"]);
    json(&b, &["commit", "-m", "b"]);

    let merged = json(&repo.root, &["merge", "a@-", "b@-"]);
    assert_eq!(merged["conflicts"][0]["path"], "a.txt");
    assert!(repo.read("a.txt").contains("<<<<<<<"));

    // Work continues; the conflict is just data.
    repo.write("other.txt", "unrelated\n");
    assert_eq!(
        json(&repo.root, &["status"])["conflicts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    repo.write("a.txt", "from a and b\n");
    assert_eq!(
        json(&repo.root, &["status"])["conflicts"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn undo_reverts_a_merge_including_files() {
    let repo = Repo::init(&[("a.txt", "base\n")]);
    json(&repo.root, &["commit", "-m", "base"]);
    let a = PathBuf::from(
        json(&repo.root, &["start", "a", "--agent", "x"])["path"]
            .as_str()
            .unwrap(),
    );
    write_in(&a, "a.txt", "changed\n");
    json(&a, &["commit", "-m", "a"]);
    let before = json(&repo.root, &["status"])["working_copy"]["commit_id"].clone();

    json(&repo.root, &["merge", "a@-"]);
    assert_eq!(repo.read("a.txt"), "changed\n");
    let undo = json(&repo.root, &["undo"]);
    assert_eq!(undo["command"], "merge");
    assert_eq!(repo.read("a.txt"), "base\n");
    assert_eq!(
        json(&repo.root, &["status"])["working_copy"]["commit_id"],
        before
    );
}

#[test]
fn undo_twice_goes_further_back() {
    let repo = Repo::init(&[]);
    json(&repo.root, &["describe", "-m", "one"]);
    json(&repo.root, &["describe", "-m", "two"]);
    json(&repo.root, &["undo"]);
    assert_eq!(
        json(&repo.root, &["status"])["working_copy"]["description"],
        "one"
    );
    json(&repo.root, &["undo"]);
    assert_eq!(
        json(&repo.root, &["status"])["working_copy"]["description"],
        ""
    );
}

#[test]
fn gc_packs_everything_and_history_stays_undoable() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    json(&repo.root, &["commit", "-m", "one"]);
    for i in 0..100 {
        repo.write(&format!("many/f{i}.txt"), &format!("{i}\n"));
    }
    json(&repo.root, &["commit", "-m", "two"]);
    repo.write("a.txt", "3\n");
    json(&repo.root, &["commit", "-m", "three"]);
    let first_op = json(&repo.root, &["op", "log"])["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["id"].as_i64().unwrap())
        .min()
        .unwrap();

    let stats = json(&repo.root, &["gc", "--prune", "now"]);
    assert!(stats["packed"].as_u64().unwrap() > 100);
    let objects = repo.root.join(".hebra/repo/objects");
    let loose = fs::read_dir(&objects)
        .unwrap()
        .flatten()
        .filter(|d| d.file_name().len() == 2)
        .count();
    assert_eq!(loose, 0, "everything is in the pack");

    let log = json(&repo.root, &["log"]);
    assert!(log.to_string().contains("two"));
    json(&repo.root, &["undo"]);
    assert_eq!(repo.read("a.txt"), "3\n");
    json(&repo.root, &["op", "restore", &first_op.to_string()]);
    assert!(!repo.root.join("many").exists());
    // And a second gc on a packed repository changes nothing it needs.
    json(&repo.root, &["gc", "--prune", "now"]);
    json(&repo.root, &["new", "@-"]);
}

#[test]
fn provenance_comes_from_the_environment() {
    let repo = Repo::init(&[]);
    let out = hb_command(&repo.root)
        .args(["--json", "describe", "-m", "by an agent"])
        .env("CLAUDECODE", "1")
        .env("HB_TASK", "TASK-42")
        .output()
        .unwrap();
    assert!(out.status.success());
    repo.write("x.txt", "x\n");
    let out = hb_command(&repo.root)
        .args(["--json", "commit", "-m", "agent work"])
        .env("CLAUDECODE", "1")
        .env("HB_TASK", "TASK-42")
        .env("HB_SESSION", "sess-1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let shown = json(&repo.root, &["show", "@-"]);
    assert_eq!(shown["provenance"]["task"], "TASK-42");
    assert_eq!(shown["provenance"]["session"], "sess-1");
    let ops = json(&repo.root, &["op", "log", "-n", "1"]);
    assert_eq!(ops["operations"][0]["actor"]["name"], "claude-code");
    assert_eq!(ops["operations"][0]["actor"]["on_behalf_of"], "ana");
}

#[test]
fn bookmarks_and_revisions_resolve() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    json(&repo.root, &["commit", "-m", "first"]);
    json(&repo.root, &["bookmark", "set", "main"]);
    let shown = json(&repo.root, &["show", "main"]);
    assert_eq!(shown["description"], "first");
    let out = run(&repo.root, &["--json", "show", "nosuchthing"]);
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["code"], "revision_not_found");
}

#[test]
fn claims_expire_and_release() {
    let repo = Repo::init(&[]);
    json(&repo.root, &["claim", "src/auth", "--ttl", "10m"]);
    assert_eq!(
        json(&repo.root, &["claims"])["claims"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(json(&repo.root, &["release"])["released"], 1);
    assert_eq!(
        json(&repo.root, &["claims"])["claims"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn mcp_server_lists_and_calls_tools() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    let mut child = hb_command(&repo.root)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"hb_status","arguments":{}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"hb_show","arguments":{"revision":"nope"}}}"#,
    ];
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    let responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 4, "notifications get no response");
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "hebra");
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "hb_start"));
    assert_eq!(responses[2]["result"]["isError"], false);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["changes"][0]["path"],
        "a.txt"
    );
    assert_eq!(responses[3]["result"]["isError"], true);
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn git_export_is_deterministic_and_keeps_provenance() {
    let repo = Repo::init(&[("a.txt", "base\n")]);
    json(&repo.root, &["commit", "-m", "base"]);
    let a = PathBuf::from(
        json(&repo.root, &["start", "feature a", "--agent", "agent-a"])["path"]
            .as_str()
            .unwrap(),
    );
    let b = PathBuf::from(
        json(&repo.root, &["start", "feature b", "--agent", "agent-b"])["path"]
            .as_str()
            .unwrap(),
    );
    write_in(&a, "a.txt", "base\nfrom a\n");
    write_in(&b, "b.txt", "from b\n");
    json(&a, &["commit", "-m", "work a"]);
    json(&b, &["commit", "-m", "work b"]);
    json(&repo.root, &["merge", "feature-a@-", "feature-b@-"]);
    json(&repo.root, &["commit", "-m", "merge both"]);

    let target = repo.root.parent().unwrap().join("export.git");
    let target_str = target.to_str().unwrap();
    let first = json(&repo.root, &["git", "export", target_str]);
    assert_eq!(first["commits"], 4);

    // Files and history arrive in Git.
    assert_eq!(git_out(&target, &["show", "main:a.txt"]), "base\nfrom a");
    assert_eq!(git_out(&target, &["show", "main:b.txt"]), "from b");
    let subjects = git_out(&target, &["log", "--format=%s", "main"]);
    assert!(subjects.starts_with("merge both"), "{subjects}");
    let parents = git_out(&target, &["log", "-1", "--format=%P", "main"]);
    assert_eq!(parents.split(' ').count(), 2, "merge keeps both parents");

    // Provenance travels as trailers.
    let body = git_out(&target, &["log", "--format=%B", "main"]);
    assert!(body.contains("Hebra-Agent: agent-a"), "{body}");
    assert!(body.contains("Hebra-On-Behalf-Of: ana"), "{body}");
    assert!(body.contains("Hebra-Task: feature b"), "{body}");

    // Exporting again produces exactly the same Git commit.
    let second = json(&repo.root, &["git", "export", target_str]);
    assert_eq!(first["git_commit"], second["git_commit"]);
}

#[test]
fn git_export_rejects_bad_branch_names() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    json(&repo.root, &["commit", "-m", "one"]);
    let target = repo.root.parent().unwrap().join("bad.git");
    let out = run(
        &repo.root,
        &[
            "--json",
            "git",
            "export",
            target.to_str().unwrap(),
            "--branch",
            "a..b",
        ],
    );
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["code"], "invalid_argument");
}

#[test]
fn git_round_trip_keeps_change_ids_and_provenance() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    json(&repo.root, &["commit", "-m", "first"]);
    let ws = PathBuf::from(
        json(&repo.root, &["start", "oauth", "--agent", "claude-code"])["path"]
            .as_str()
            .unwrap(),
    );
    write_in(&ws, "b.txt", "2\n");
    let done = json(&ws, &["commit", "-m", "add b"]);
    let change = done["committed"]["change_id"].as_str().unwrap().to_owned();

    let git_dir = repo.root.parent().unwrap().join("round.git");
    json(&ws, &["git", "export", git_dir.to_str().unwrap()]);

    let other = Repo::new();
    json(&other.root, &["init"]);
    let imported = json(&other.root, &["git", "import", git_dir.to_str().unwrap()]);
    assert_eq!(imported["stats"]["commits"], 2);
    let tip = &imported["tip"];
    assert_eq!(tip["change_id"], change.as_str());
    assert_eq!(tip["description"], "add b");
    assert_eq!(tip["author"]["name"], "claude-code");
    assert_eq!(tip["author"]["kind"], "agent");
    assert_eq!(tip["provenance"]["task"], "oauth");

    // Importing again is a no-op: same commits, same ids.
    let again = json(&other.root, &["git", "import", git_dir.to_str().unwrap()]);
    assert_eq!(again["tip"]["commit_id"], tip["commit_id"]);

    // And the history can be worked on directly.
    json(&other.root, &["new", "main"]);
    assert_eq!(other.read("b.txt"), "2\n");
}

#[test]
fn whoever_fills_an_empty_change_becomes_its_author() {
    let repo = Repo::init(&[("a.txt", "1\n")]);
    json(&repo.root, &["commit", "-m", "base"]);
    // The empty working-copy change was created by the human...
    repo.write("a.txt", "2\n");
    // ...but an agent writes into it and commits.
    let out = hb_command(&repo.root)
        .args(["--json", "commit", "-m", "agent work"])
        .env("HB_ACTOR", "codex")
        .env("HB_ACTOR_KIND", "agent")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let shown = json(&repo.root, &["show", "@-"]);
    assert_eq!(shown["author"]["name"], "codex");
    assert_eq!(shown["author"]["kind"], "agent");
}
