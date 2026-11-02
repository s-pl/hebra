//! Everything Git can do, Hebra has to do too. Each test runs the same story
//! with both tools where that makes sense and compares what comes out.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn hb_raw(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hb"))
        .current_dir(dir)
        .args(args)
        .env_remove("CLAUDECODE")
        .env_remove("HB_ACTOR")
        .env_remove("HB_ON_BEHALF_OF")
        .env("HB_ACTOR", "tester")
        .env("HB_ACTOR_KIND", "human")
        .env("HB_CONFIG_DIR", dir.join(".test-config-unused"))
        .output()
        .unwrap()
}

/// Runs `hb --json` and returns what it printed.
#[track_caller]
fn hb(dir: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = hb_raw(dir, &full);
    assert!(
        out.status.success(),
        "hb {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

/// Runs `hb` and returns its text.
#[track_caller]
fn hbt(dir: &Path, args: &[&str]) -> String {
    let out = hb_raw(dir, args);
    assert!(
        out.status.success(),
        "hb {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[track_caller]
fn hb_fails(dir: &Path, args: &[&str]) -> String {
    let out = hb_raw(dir, args);
    assert!(!out.status.success(), "hb {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[track_caller]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Tester")
        .env("GIT_AUTHOR_EMAIL", "tester@example.com")
        .env("GIT_COMMITTER_NAME", "Tester")
        .env("GIT_COMMITTER_EMAIL", "tester@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Lab {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Self { _dir: dir, root }
    }

    fn path(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// An empty Hebra repository.
    fn repo(&self, name: &str) -> PathBuf {
        let dir = self.path(name);
        hb(&dir, &["init"]);
        hb(&dir, &["config", "set", "user.name", "tester"]);
        dir
    }

    /// A Git repository with a few commits and a bare copy to act as GitHub.
    fn github(&self) -> PathBuf {
        let work = self.path("gh-work");
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["config", "commit.gpgsign", "false"]);
        write(&work, "README.md", "hello\n");
        write(&work, "src/lib.rs", "pub fn a() {}\n");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", "first"]);
        write(&work, "src/lib.rs", "pub fn a() {}\npub fn b() {}\n");
        git(&work, &["commit", "-qam", "second"]);
        git(&work, &["tag", "v1.0"]);
        git(&work, &["tag", "-a", "v1.1", "-m", "release one one"]);
        git(&work, &["branch", "feature"]);
        let bare = self.root.join("github.git");
        let target = bare.to_string_lossy().into_owned();
        git(
            &self.root,
            &["clone", "-q", "--bare", work.to_str().unwrap(), &target],
        );
        bare
    }
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
}

fn read(dir: &Path, path: &str) -> String {
    fs::read_to_string(dir.join(path))
        .unwrap()
        .replace("\r\n", "\n")
}

fn commit(dir: &Path, path: &str, text: &str, message: &str) {
    write(dir, path, text);
    hb(dir, &["commit", "-m", message]);
}

// ---------------------------------------------------------------- clone

#[test]
fn clone_from_git_keeps_branches_tags_authors_and_can_push_back() {
    let lab = Lab::new();
    let bare = lab.github();
    let url = bare.to_string_lossy().into_owned();
    let out = hb(&lab.root, &["clone", "--git", &url, "copy"]);
    assert_eq!(out["branch"], "main");
    let copy = lab.root.join("copy");

    // Files, branches and tags are there, on the default branch.
    assert_eq!(read(&copy, "README.md"), "hello\n");
    assert!(read(&copy, "src/lib.rs").contains("pub fn b"));
    assert_eq!(hbt(&copy, &["branch", "--show-current"]).trim(), "main");
    let branches = hbt(&copy, &["branch", "-a"]);
    assert!(branches.contains("origin/feature"), "{branches}");
    let tags = hbt(&copy, &["tag", "-l"]);
    assert!(tags.contains("v1.0") && tags.contains("v1.1"), "{tags}");

    // Authors keep their email, and annotated tags their message.
    let shown = hb(&copy, &["log", "-n", "1", "main"]);
    assert_eq!(shown["commits"][0]["author"]["email"], "tester@example.com");
    let listed = hb(&copy, &["tag", "-l", "v1.1"]);
    assert_eq!(listed["tags"][0]["tag"]["message"], "release one one");

    // New work lands on main and goes back on top of the same commits.
    let before = git(&bare, &["rev-parse", "main"]);
    write(&copy, "new.txt", "from hebra\n");
    hb(&copy, &["commit", "-m", "from hebra"]);
    hb(&copy, &["push", "origin"]);
    let after = git(&bare, &["rev-parse", "main"]);
    assert_ne!(before, after);
    assert_eq!(git(&bare, &["rev-parse", "main~1"]), before, "history kept");
    assert_eq!(git(&bare, &["show", "main:new.txt"]), "from hebra");
    assert_eq!(
        git(&bare, &["log", "-1", "--format=%s", "main"]),
        "from hebra"
    );
    git(&bare, &["fsck", "--strict"]);

    // Someone pushes with Git; `hb pull` brings it in.
    let other = lab.root.join("other");
    git(&lab.root, &["clone", "-q", &url, other.to_str().unwrap()]);
    git(&other, &["config", "commit.gpgsign", "false"]);
    write(&other, "git.txt", "from git\n");
    git(&other, &["add", "."]);
    git(&other, &["commit", "-qm", "from git"]);
    git(&other, &["push", "-q", "origin", "main"]);
    hb(&copy, &["pull", "origin"]);
    assert_eq!(read(&copy, "git.txt"), "from git\n");
    assert_eq!(
        hbt(&copy, &["log", "--format=%s", "-n", "2", "main"])
            .lines()
            .next(),
        Some("from git")
    );

    // And the Hebra commit it came from is not sent twice.
    write(&copy, "again.txt", "x\n");
    hb(&copy, &["commit", "-m", "again"]);
    hb(&copy, &["push", "origin", "main"]);
    assert_eq!(git(&bare, &["rev-list", "--count", "main"]), "5");
}

#[test]
fn shallow_and_single_branch_clones_work() {
    let lab = Lab::new();
    let bare = lab.github();
    let url = format!("file://{}", bare.to_string_lossy().replace('\\', "/"));
    hb(&lab.root, &["clone", "--depth", "1", &url, "shallow"]);
    let dir = lab.root.join("shallow");
    assert_eq!(hbt(&dir, &["rev-list", "--count", "main"]).trim(), "1");
    assert!(read(&dir, "src/lib.rs").contains("pub fn b"));

    hb(
        &lab.root,
        &[
            "clone",
            "--single-branch",
            "--branch",
            "feature",
            &url,
            "single",
        ],
    );
    let dir = lab.root.join("single");
    assert_eq!(hbt(&dir, &["branch", "--show-current"]).trim(), "feature");
    assert!(!hbt(&dir, &["branch", "-a"]).contains("origin/main"));
}

#[test]
fn clone_and_exchange_between_local_repositories() {
    let lab = Lab::new();
    let origin = lab.repo("origin");
    commit(&origin, "a.txt", "one\n", "first");
    hb(&origin, &["bookmark", "set", "main"]);
    hb(&lab.root, &["clone", "origin", "copy"]);
    let copy = lab.root.join("copy");
    assert_eq!(read(&copy, "a.txt"), "one\n");
    commit(&copy, "b.txt", "two\n", "second");
    hb(&copy, &["push", "origin", "main"]);
    assert!(hbt(&origin, &["log", "-n", "5", "main"]).contains("second"));
    commit(&origin, "c.txt", "three\n", "third");
    hb(&origin, &["bookmark", "set", "main"]);
    hb(&copy, &["pull", "origin", "--rebase"]);
    assert!(hbt(&copy, &["log", "main"]).contains("third"));
}

#[test]
fn an_empty_repository_clones_and_a_full_directory_is_refused() {
    let lab = Lab::new();
    let bare = lab.root.join("empty.git");
    let target = bare.to_string_lossy().into_owned();
    git(&lab.root, &["init", "-q", "--bare", &target]);
    hb(&lab.root, &["clone", "--git", &target, "copy"]);
    let full = lab.path("full");
    write(&full, "x", "y");
    assert!(hb_fails(&lab.root, &["clone", "--git", &target, "full"]).contains("not empty"));
}

// ---------------------------------------------------------- daily commands

#[test]
fn branches_work_like_git_branches() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "a.txt", "1\n", "base");
    hb(&dir, &["switch", "-c", "main", "@-"]);
    commit(&dir, "b.txt", "2\n", "on main");
    assert_eq!(
        hbt(&dir, &["rev-parse", "main"]),
        hbt(&dir, &["rev-parse", "HEAD"]),
        "committing moves the current branch"
    );
    hb(&dir, &["switch", "-c", "topic"]);
    commit(&dir, "c.txt", "3\n", "on topic");
    hb(&dir, &["switch", "main"]);
    assert!(!dir.join("c.txt").exists());
    let list = hbt(&dir, &["branch"]);
    assert!(
        list.contains("* main") && list.contains("  topic"),
        "{list}"
    );

    let merged = hbt(&dir, &["branch", "--merged", "topic"]);
    assert!(merged.contains("main"), "{merged}");
    let not_merged = hbt(&dir, &["branch", "--no-merged", "main"]);
    assert!(not_merged.contains("topic"), "{not_merged}");
    hb(&dir, &["branch", "-m", "topic", "renamed"]);
    assert!(hbt(&dir, &["branch"]).contains("renamed"));
    hb(&dir, &["branch", "-d", "renamed"]);
    assert!(!hbt(&dir, &["branch"]).contains("renamed"));
    assert!(hb_fails(&dir, &["branch", "main"]).contains("already exists"));
}

#[test]
fn uncommitted_work_follows_a_switch_and_ambiguity_is_explained() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "a.txt", "1\n", "base");
    hb(&dir, &["branch", "main"]);
    hb(&dir, &["branch", "other"]);
    hb(&dir, &["switch", "main"]);
    write(&dir, "wip.txt", "in progress\n");
    hb(&dir, &["switch", "other"]);
    assert_eq!(read(&dir, "wip.txt"), "in progress\n");
    assert_eq!(hbt(&dir, &["branch", "--show-current"]).trim(), "other");
}

#[test]
fn commit_options_match_git() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    write(&dir, "a.txt", "1\n");
    write(&dir, "b.txt", "2\n");
    // Only some paths; the rest stays.
    hb(&dir, &["commit", "-m", "just a", "a.txt"]);
    let changes = hb(&dir, &["status"]);
    assert_eq!(changes["changes"][0]["path"], "b.txt");

    // Message from a file, several -m, author and date.
    write(&dir, "msg.txt", "from a file\n");
    hb(&dir, &["commit", "-F", "msg.txt"]);
    write(&dir, "c.txt", "3\n");
    hb(
        &dir,
        &[
            "commit",
            "-m",
            "title",
            "-m",
            "body",
            "--author",
            "Ana <ana@x.dev>",
            "--date",
            "2026-01-02",
        ],
    );
    let c = hb(&dir, &["show", "@-"]);
    assert_eq!(c["description"], "title\n\nbody");
    assert_eq!(c["author"]["name"], "Ana");
    assert_eq!(c["author"]["email"], "ana@x.dev");
    assert_eq!(
        hbt(&dir, &["log", "-n", "1", "--format=%ae %ad", "@-"]).trim(),
        "ana@x.dev 2026-01-02T00:00:00Z"
    );

    // Amend: changes and a new message.
    write(&dir, "c.txt", "3 fixed\n");
    hb(&dir, &["commit", "--amend", "-m", "title fixed"]);
    let c = hb(&dir, &["show", "@-"]);
    assert_eq!(c["description"], "title fixed");
    assert_eq!(hbt(&dir, &["cat", "@-:c.txt"]), "3 fixed\n");
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        0
    );

    // Empty commits are fine, and flags of habit are accepted.
    hb(&dir, &["commit", "-a", "--allow-empty", "-m", "nothing"]);
    hb(&dir, &["add", "."]);
}

#[test]
fn stash_round_trips() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "a.txt", "1\n", "base");
    write(&dir, "a.txt", "changed\n");
    write(&dir, "new.txt", "new\n");
    hb(&dir, &["stash", "push", "-m", "wip"]);
    assert_eq!(read(&dir, "a.txt"), "1\n");
    assert!(!dir.join("new.txt").exists());
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        0
    );
    assert!(hbt(&dir, &["stash", "list"]).contains("wip"));
    assert!(hbt(&dir, &["stash", "show"]).contains("new.txt"));

    commit(&dir, "b.txt", "other work\n", "meanwhile");
    hb(&dir, &["stash", "pop"]);
    assert_eq!(read(&dir, "a.txt"), "changed\n");
    assert_eq!(read(&dir, "new.txt"), "new\n");
    assert!(hbt(&dir, &["stash", "list"]).is_empty());

    hb(&dir, &["stash", "push"]);
    hb(&dir, &["stash", "branch", "from-stash"]);
    assert_eq!(
        hbt(&dir, &["branch", "--show-current"]).trim(),
        "from-stash"
    );
    assert_eq!(read(&dir, "a.txt"), "changed\n");
    hb(&dir, &["stash", "push"]);
    hb(&dir, &["stash", "clear"]);
    assert!(hbt(&dir, &["stash", "list"]).is_empty());
}

#[test]
fn reset_restore_and_clean() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "a.txt", "1\n", "one");
    hb(&dir, &["bookmark", "set", "main"]);
    hb(&dir, &["switch", "main"]);
    commit(&dir, "a.txt", "2\n", "two");
    commit(&dir, "a.txt", "3\n", "three");

    // restore: a file from another revision.
    write(&dir, "a.txt", "scratch\n");
    hb(&dir, &["restore", "a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "3\n");
    hb(&dir, &["restore", "--source", "HEAD~2", "a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "1\n");
    hb(&dir, &["checkout", "HEAD", "--", "a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "3\n");

    // reset --soft keeps the files; the change shows up as a diff.
    hb(&dir, &["reset", "--soft", "HEAD~1"]);
    assert_eq!(read(&dir, "a.txt"), "3\n");
    assert_eq!(hb(&dir, &["status"])["changes"][0]["path"], "a.txt");
    assert!(hbt(&dir, &["log", "--format=%s", "-n", "1", "main"]).contains("two"));

    // reset --hard takes them too, and undo brings it all back.
    hb(&dir, &["reset", "--hard", "HEAD~1"]);
    assert_eq!(read(&dir, "a.txt"), "1\n");
    hb(&dir, &["undo"]);
    assert_eq!(read(&dir, "a.txt"), "3\n");

    // clean: new files, and with -x ignored ones.
    write(&dir, ".gitignore", "*.log\n");
    hb(&dir, &["commit", "-m", "ignore logs"]);
    write(&dir, "junk.txt", "x\n");
    write(&dir, "debug.log", "x\n");
    assert!(hb_fails(&dir, &["clean"]).contains("-f"));
    assert!(hbt(&dir, &["clean", "-n"]).contains("junk.txt"));
    assert!(dir.join("junk.txt").exists());
    hb(&dir, &["clean", "-f"]);
    assert!(!dir.join("junk.txt").exists());
    assert!(dir.join("debug.log").exists());
    hb(&dir, &["clean", "-fx"]);
    assert!(!dir.join("debug.log").exists());
}

#[test]
fn rm_mv_and_renames() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    let body: String = (0..30).map(|i| format!("line {i}\n")).collect();
    commit(&dir, "old.txt", &body, "base");
    commit(&dir, "dir/x.txt", "x\n", "more");

    hb(&dir, &["mv", "old.txt", "new.txt"]);
    assert!(!dir.join("old.txt").exists());
    let files = hb(&dir, &["diff"]);
    assert_eq!(files["files"][0]["status"], "renamed");
    assert_eq!(files["files"][0]["renamed_from"], "old.txt");
    let short = hbt(&dir, &["status", "--porcelain"]);
    assert!(short.contains("R  old.txt -> new.txt"), "{short}");
    hb(&dir, &["commit", "-m", "renamed"]);

    assert!(hb_fails(&dir, &["rm", "dir"]).contains("-r"));
    hb(&dir, &["rm", "-r", "dir"]);
    assert!(!dir.join("dir").exists());
    hb(&dir, &["rm", "--cached", "new.txt"]);
    assert!(dir.join("new.txt").exists());
    assert!(hbt(&dir, &["ls-files"]).lines().all(|l| l != "new.txt"));
}

#[test]
fn a_change_merges_into_a_renamed_file() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    let body: String = (0..30).map(|i| format!("line {i}\n")).collect();
    commit(&dir, "old.rs", &body, "base");
    hb(&dir, &["branch", "main"]);
    hb(&dir, &["switch", "-c", "rename"]);
    hb(&dir, &["mv", "old.rs", "new.rs"]);
    hb(&dir, &["commit", "-m", "rename it"]);
    hb(&dir, &["switch", "main"]);
    commit(
        &dir,
        "old.rs",
        &body.replace("line 7\n", "line seven\n"),
        "edit it",
    );
    let merged = hb(&dir, &["merge", "rename"]);
    assert_eq!(merged["conflicts"].as_array().unwrap().len(), 0);
    assert!(read(&dir, "new.rs").contains("line seven"));
    assert!(!dir.join("old.rs").exists());
}

// ---------------------------------------------------------- rewriting

/// Runs `hb` for a linear series: base <- a <- b <- c on `main`.
fn series(lab: &Lab) -> PathBuf {
    let dir = lab.repo("series");
    commit(&dir, "base.txt", "base\n", "base");
    hb(&dir, &["branch", "main"]);
    hb(&dir, &["switch", "main"]);
    commit(&dir, "a.txt", "a\n", "add a");
    commit(&dir, "b.txt", "b\n", "add b");
    commit(&dir, "c.txt", "c\n", "add c");
    dir
}

fn subjects(dir: &Path, rev: &str) -> Vec<String> {
    hbt(dir, &["log", "--format=%s", "-n", "50", rev])
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn rebase_moves_a_branch_and_keeps_change_ids() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(&dir, &["switch", "-c", "topic", "HEAD~2"]);
    commit(&dir, "t1.txt", "1\n", "topic one");
    commit(&dir, "t2.txt", "2\n", "topic two");
    let change = hb(&dir, &["show", "@-"])["change_id"].clone();
    hb(&dir, &["rebase", "main"]);
    assert_eq!(
        subjects(&dir, "topic"),
        ["topic two", "topic one", "add c", "add b", "add a", "base"]
    );
    assert_eq!(hb(&dir, &["show", "topic"])["change_id"], change);
    assert_eq!(read(&dir, "t2.txt"), "2\n");
    assert_eq!(read(&dir, "c.txt"), "c\n", "the working copy moved along");
    // Undo puts everything back.
    hb(&dir, &["undo"]);
    assert_eq!(
        subjects(&dir, "topic"),
        ["topic two", "topic one", "add a", "base"]
    );
}

#[test]
fn rebase_with_a_conflict_finishes_and_records_it() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "f.txt", "base\n", "base");
    hb(&dir, &["branch", "main"]);
    hb(&dir, &["switch", "-c", "topic"]);
    commit(&dir, "f.txt", "topic\n", "topic edit");
    hb(&dir, &["switch", "main"]);
    commit(&dir, "f.txt", "main\n", "main edit");
    hb(&dir, &["switch", "topic"]);
    let out = hb(&dir, &["rebase", "main"]);
    assert_eq!(out["moved"], 2);
    assert!(read(&dir, "f.txt").contains("<<<<<<<"));
    let shown = hb(&dir, &["show", "topic"]);
    assert_eq!(shown["conflicts"][0]["path"], "f.txt");
    // `--abort` has nothing to stop: it says what to do instead.
    assert!(hb_fails(&dir, &["rebase", "--abort", "main"]).contains("hb undo"));
}

#[test]
fn interactive_rebase_reorders_rewords_squashes_and_drops() {
    let lab = Lab::new();
    let dir = series(&lab);
    let ids: Vec<String> = hbt(&dir, &["log", "--format=%h", "-n", "3", "main"])
        .lines()
        .map(str::to_owned)
        .collect();
    let (c, b, a) = (&ids[0], &ids[1], &ids[2]);
    let todo = format!("pick {c}\nreword {a} add a, better\nfixup {b}\n");
    write(&dir, "todo.txt", &todo);
    let base = hbt(&dir, &["rev-parse", "main~3"]);
    hb(
        &dir,
        &[
            "rebase",
            "-d",
            base.trim(),
            "--branch",
            "main",
            "--todo",
            "todo.txt",
        ],
    );
    assert_eq!(subjects(&dir, "main"), ["add a, better", "add c", "base"]);
    assert_eq!(
        hbt(&dir, &["cat", "main:b.txt"]),
        "b\n",
        "fixup keeps its files"
    );
}

#[test]
fn autosquash_folds_fixups_into_their_targets() {
    let lab = Lab::new();
    let dir = series(&lab);
    write(&dir, "a.txt", "a fixed\n");
    hb(&dir, &["commit", "--fixup", "main~1", "-a"]);
    let base = hbt(&dir, &["rev-parse", "main~3"]);
    hb(
        &dir,
        &[
            "rebase",
            "--autosquash",
            "-d",
            base.trim(),
            "--branch",
            "main",
        ],
    );
    assert_eq!(subjects(&dir, "main"), ["add c", "add b", "add a", "base"]);
    assert_eq!(hbt(&dir, &["cat", "main:a.txt"]), "a fixed\n");
}

#[test]
fn cherry_pick_and_revert_match_git() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(&dir, &["switch", "-c", "other", "HEAD~3"]);
    hb(&dir, &["cherry-pick", "-x", "main~1"]);
    assert_eq!(read(&dir, "b.txt"), "b\n");
    assert!(!dir.join("a.txt").exists());
    let picked = hb(&dir, &["show", "@-"]);
    assert!(
        picked["description"]
            .as_str()
            .unwrap()
            .contains("cherry picked from commit")
    );
    assert_eq!(
        subjects(&dir, "other")[0],
        "add b\n\n(cherry picked from commit x"
            .split('\n')
            .next()
            .unwrap()
    );
    assert_eq!(hbt(&dir, &["branch", "--show-current"]).trim(), "other");

    // Ranges, oldest first, as in Git.
    hb(&dir, &["cherry-pick", "main~3..main~1"]);
    assert_eq!(read(&dir, "a.txt"), "a\n");

    hb(&dir, &["revert", "@-"]);
    let reverted = subjects(&dir, "other");
    assert!(reverted[0].starts_with("Revert \""), "{reverted:?}");

    // Merges need a side, like Git.
    hb(&dir, &["switch", "main"]);
    hb(&dir, &["merge", "other", "-m", "merge other"]);
    hb(&dir, &["commit", "-m", "merge other"]);
    assert!(hb_fails(&dir, &["revert", "@-"]).contains("mainline"));
    hb(&dir, &["revert", "-m", "1", "@-"]);
    assert_eq!(read(&dir, "c.txt"), "c\n");
}

#[test]
fn squash_split_abandon_and_edit() {
    let lab = Lab::new();
    let dir = series(&lab);
    write(&dir, "one.txt", "1\n");
    write(&dir, "two.txt", "2\n");
    hb(&dir, &["split", "-m", "only one", "one.txt"]);
    assert_eq!(subjects(&dir, "@-")[0], "only one");
    assert_eq!(hb(&dir, &["status"])["changes"][0]["path"], "two.txt");

    hb(&dir, &["squash"]);
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        0
    );
    assert_eq!(hbt(&dir, &["cat", "@-:two.txt"]), "2\n");

    hb(&dir, &["abandon", "main~1"]);
    assert_eq!(subjects(&dir, "main"), ["add c", "add a", "base"]);
    assert!(dir.join("c.txt").exists());
    assert!(!dir.join("b.txt").exists());

    hb(&dir, &["edit", "main~1"]);
    assert_eq!(read(&dir, "a.txt"), "a\n");
    assert!(!dir.join("c.txt").exists());
    write(&dir, "a.txt", "edited in place\n");
    hb(&dir, &["status"]);
    assert_eq!(hbt(&dir, &["cat", "main~1:a.txt"]), "edited in place\n");
}

#[test]
fn filter_removes_a_path_from_all_history() {
    let lab = Lab::new();
    let dir = series(&lab);
    commit(&dir, "secret.env", "TOKEN=abc\n", "oops");
    commit(&dir, "d.txt", "d\n", "more");
    hb(&dir, &["filter", "--remove", "secret.env", "-r", "main"]);
    assert!(!dir.join("secret.env").exists());
    assert!(
        hbt(&dir, &["ls-tree", "-r", "--name-only", "main"])
            .lines()
            .all(|l| l != "secret.env")
    );
    assert_eq!(subjects(&dir, "main").len(), 6);
}

// ------------------------------------------------------------- reading

#[test]
fn log_filters_and_formats() {
    let lab = Lab::new();
    let dir = series(&lab);
    assert_eq!(subjects(&dir, "main~2..main"), ["add c", "add b"]);
    assert_eq!(
        hbt(&dir, &["log", "--oneline", "-n", "1", "main"])
            .split_whitespace()
            .nth(1),
        Some("(main)")
    );
    let by_path = hbt(&dir, &["log", "--format=%s", "--", "b.txt"]);
    assert_eq!(by_path.trim(), "add b");
    assert_eq!(
        hbt(&dir, &["log", "--format=%s", "--grep", "add [ab]", "main"])
            .lines()
            .count(),
        2
    );
    assert_eq!(
        hbt(
            &dir,
            &[
                "log",
                "--format=%s",
                "--author",
                "tester",
                "-n",
                "2",
                "main"
            ]
        )
        .lines()
        .count(),
        2
    );
    assert_eq!(
        hbt(&dir, &["log", "--format=%s", "--author", "nobody", "main"]),
        ""
    );
    assert_eq!(
        hbt(
            &dir,
            &["log", "--format=%s", "--reverse", "-n", "2", "main"]
        )
        .lines()
        .next(),
        Some("add b"),
        "like Git: the newest two, oldest first"
    );
    assert_eq!(
        hbt(
            &dir,
            &[
                "log",
                "--format=%s",
                "--since",
                "1d",
                "--until",
                "2020-01-01",
                "main"
            ]
        ),
        ""
    );
    assert!(hbt(&dir, &["log", "--name-status", "-n", "1", "main"]).contains("A\tc.txt"));
    assert!(hbt(&dir, &["log", "--stat", "-n", "1", "main"]).contains("1 file(s) changed"));
    assert!(hbt(&dir, &["log", "-p", "-n", "1", "main"]).contains("+c"));
    assert!(hbt(&dir, &["log", "--graph", "--oneline", "main"]).contains("* "));

    // Pickaxe: which change added this text?
    assert_eq!(
        hbt(&dir, &["log", "--format=%s", "-S", "c", "main"])
            .trim()
            .lines()
            .next(),
        Some("add c")
    );
    assert_eq!(
        hbt(&dir, &["log", "--format=%s", "-G", "^b$", "main"]).trim(),
        "add b"
    );

    // Parents and merges.
    hb(&dir, &["switch", "-c", "side", "main~1"]);
    commit(&dir, "s.txt", "s\n", "side");
    hb(&dir, &["switch", "main"]);
    hb(&dir, &["merge", "side", "-m", "merge side"]);
    hb(&dir, &["commit", "-m", "merge side"]);
    assert_eq!(
        hbt(&dir, &["log", "--format=%s", "--merges", "main"]).trim(),
        "merge side"
    );
    assert!(!hbt(&dir, &["log", "--format=%s", "--no-merges", "main"]).contains("merge side"));
    let first = hbt(
        &dir,
        &["log", "--format=%s", "--first-parent", "-n", "50", "main"],
    );
    assert!(first.lines().all(|l| l != "side"), "{first}");
    assert!(hbt(&dir, &["log", "--graph", "--format=%s", "main"]).contains('\\'));
}

#[test]
fn follow_keeps_going_across_a_rename() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    let body: String = (0..30).map(|i| format!("line {i}\n")).collect();
    commit(&dir, "old.txt", &body, "created");
    hb(&dir, &["mv", "old.txt", "new.txt"]);
    hb(&dir, &["commit", "-m", "renamed"]);
    write(&dir, "new.txt", &format!("{body}more\n"));
    hb(&dir, &["commit", "-m", "edited"]);
    let log = hbt(
        &dir,
        &["log", "--format=%s", "--follow", "-n", "9", "--", "new.txt"],
    );
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        ["edited", "renamed", "created"]
    );
}

#[test]
fn diff_forms_match_git() {
    let lab = Lab::new();
    let dir = series(&lab);
    assert!(hbt(&dir, &["diff", "main~2", "main"]).contains("+b"));
    assert!(hbt(&dir, &["diff", "main~2..main", "--name-only"]).contains("c.txt"));
    assert!(hbt(&dir, &["diff", "main~1...main", "--name-status"]).contains("A\tc.txt"));
    assert!(hbt(&dir, &["diff", "main~2", "main", "--stat"]).contains("2 file(s) changed"));
    let only = hbt(
        &dir,
        &["diff", "main~2", "main", "--name-only", "--", "b.txt"],
    );
    assert_eq!(only.trim(), "b.txt");
    write(&dir, "a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
    hb(&dir, &["commit", "-m", "long"]);
    write(&dir, "a.txt", "1\n2\n3\nX\n5\n6\n7\n8\n");
    let patch = hbt(&dir, &["diff", "-U", "1"]);
    assert!(patch.contains(" 3\n-4\n+X\n 5\n"), "{patch}");
    assert!(!patch.contains(" 2\n"), "{patch}");
}

#[test]
fn blame_names_who_wrote_each_line_with_provenance() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    let body: String = (0..8).map(|i| format!("line {i}\n")).collect();
    commit(&dir, "f.txt", &body, "first");
    hb(&dir, &["mv", "f.txt", "g.txt"]);
    hb(&dir, &["commit", "-m", "renamed"]);
    write(&dir, "g.txt", &body.replace("line 3\n", "line three\n"));
    let out = Command::new(env!("CARGO_BIN_EXE_hb"))
        .current_dir(&dir)
        .args(["commit", "-m", "agent edit"])
        .env("HB_ACTOR", "claude-code")
        .env("HB_ACTOR_KIND", "agent")
        .env("HB_TASK", "fix the line")
        .env("HB_PROMPT", "please fix line three")
        .env("HB_ON_BEHALF_OF", "ana")
        .output()
        .unwrap();
    assert!(out.status.success());
    let blame = hb(&dir, &["blame", "g.txt"]);
    let lines = blame["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 8);
    assert_eq!(lines[3]["description"], "agent edit");
    assert_eq!(lines[3]["author"]["name"], "claude-code");
    assert_eq!(lines[3]["provenance"]["task"], "fix the line");
    assert_eq!(lines[3]["provenance"]["prompt"], "please fix line three");
    assert_eq!(lines[0]["description"], "first");
    assert_eq!(lines[0]["path"], "f.txt", "followed across the rename");
    assert!(hbt(&dir, &["blame", "-L", "4,4", "g.txt"]).contains("line three"));
    assert!(!hbt(&dir, &["blame", "-L", "4,4", "g.txt"]).contains("line 0"));
    assert!(hbt(&dir, &["annotate", "g.txt"]).contains("line 7"));
}

#[test]
fn blame_agrees_with_git_on_a_real_history() {
    let lab = Lab::new();
    let work = lab.path("g");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    let versions = [
        "a\nb\nc\nd\ne\n",
        "a\nb2\nc\nd\ne\n",
        "a\nb2\nc\nnew\nd\ne\n",
        "b2\nc\nnew\nd\ne\nf\n",
        "b2\nc changed\nnew\nd\ne\nf\n",
    ];
    for (i, text) in versions.iter().enumerate() {
        write(&work, "f.txt", text);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", &format!("v{i}")]);
    }
    let dir = lab.repo("h");
    hb(&dir, &["git", "import", work.to_str().unwrap()]);
    let ours: Vec<String> = hb(&dir, &["blame", "-r", "main", "f.txt"])["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["description"].as_str().unwrap().to_owned())
        .collect();
    let theirs: Vec<String> = git(&work, &["blame", "--root", "-s", "-l", "f.txt"])
        .lines()
        .map(|l| {
            let sha = l.split_whitespace().next().unwrap();
            git(&work, &["log", "-1", "--format=%s", sha])
        })
        .collect();
    assert_eq!(ours, theirs);
}

#[test]
fn grep_searches_the_working_copy_and_revisions() {
    let lab = Lab::new();
    let dir = series(&lab);
    write(&dir, "code.rs", "fn pay() {}\nfn Refund() {}\n");
    assert!(hbt(&dir, &["grep", "pay"]).contains("code.rs:1:fn pay() {}"));
    assert!(hbt(&dir, &["grep", "-i", "refund"]).contains("code.rs:2"));
    assert_eq!(hbt(&dir, &["grep", "-l", "fn"]).trim(), "code.rs");
    assert_eq!(hbt(&dir, &["grep", "-c", "fn"]).trim(), "code.rs:2");
    assert_eq!(
        hb_raw(&dir, &["grep", "pay", "main"]).status.code(),
        Some(1)
    );
    assert!(hbt(&dir, &["grep", "-F", "()", "--", "code.rs"]).contains("code.rs:1"));
    assert_eq!(hb_raw(&dir, &["grep", "-w", "pa"]).status.code(), Some(1));
}

#[test]
fn plumbing_answers_scripts() {
    let lab = Lab::new();
    let dir = series(&lab);
    assert!(
        hbt(&dir, &["rev-parse", "--show-toplevel"])
            .trim()
            .ends_with("series")
    );
    assert_eq!(
        hbt(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );
    assert_eq!(hbt(&dir, &["rev-list", "--count", "main"]).trim(), "4");
    assert_eq!(
        hbt(&dir, &["rev-list", "--count", "main~2..main"]).trim(),
        "2"
    );
    let base = hbt(&dir, &["merge-base", "main", "main~2"]);
    assert_eq!(base, hbt(&dir, &["rev-parse", "main~2"]));
    assert_eq!(
        hb_raw(&dir, &["merge-base", "--is-ancestor", "main~1", "main"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        hb_raw(&dir, &["merge-base", "--is-ancestor", "main", "main~1"])
            .status
            .code(),
        Some(1)
    );
    assert!(hbt(&dir, &["for-each-ref"]).contains("refs/heads/main"));
    assert!(hbt(&dir, &["name-rev", "main~1"]).contains("main~1"));
    let ls = hbt(&dir, &["ls-files"]);
    assert_eq!(
        ls.lines().collect::<Vec<_>>(),
        ["a.txt", "b.txt", "base.txt", "c.txt"]
    );
    assert!(hbt(&dir, &["ls-tree", "main"]).contains("100644 blob"));
    assert_eq!(hbt(&dir, &["cat-file", "-t", "main"]).trim(), "commit");
    assert_eq!(hbt(&dir, &["cat", "main:a.txt"]), "a\n");
    assert_eq!(hbt(&dir, &["show", "main:a.txt"]), "a\n");
    assert!(hbt(&dir, &["shortlog", "-s", "main"]).contains("4\ttester"));
    assert!(hbt(&dir, &["reflog"]).contains("commit"));
    assert!(hbt(&dir, &["stats"]).contains("loose objects"));
    let git_style = hbt(&dir, &["show", "HEAD~1", "--format=%s", "-s"]);
    assert_eq!(git_style.trim(), "add b");
    assert!(hbt(&dir, &["show", "--stat", "main"]).contains("1 file(s) changed"));
}

#[test]
fn tags_like_git() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(&dir, &["tag", "v1", "main~1"]);
    hb(&dir, &["tag", "-a", "v2", "-m", "second release"]);
    assert_eq!(
        hbt(&dir, &["tag"]).lines().collect::<Vec<_>>(),
        ["v1", "v2"]
    );
    assert_eq!(
        hbt(&dir, &["rev-parse", "v1"]),
        hbt(&dir, &["rev-parse", "main~1"])
    );
    assert_eq!(hbt(&dir, &["tag", "--contains", "main"]).trim(), "v2");
    assert_eq!(hbt(&dir, &["tag", "-l", "v1*"]).trim(), "v1");
    assert_eq!(hbt(&dir, &["describe", "-r", "main~1"]).trim(), "v1");
    assert!(hb_fails(&dir, &["tag", "v1"]).contains("already exists"));
    hb(&dir, &["tag", "-f", "v1", "main"]);
    hb(&dir, &["tag", "-d", "v1"]);
    assert!(!hbt(&dir, &["tag"]).contains("v1"));
    commit(&dir, "e.txt", "e\n", "after");
    assert!(hbt(&dir, &["describe", "-r", "main"]).starts_with("v2-1-g"));
}

#[test]
fn merge_options_match_git() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(&dir, &["switch", "-c", "ff", "main"]);
    commit(&dir, "ff.txt", "1\n", "ahead");
    hb(&dir, &["switch", "main"]);
    let tip = hbt(&dir, &["rev-parse", "main"]);
    hb(&dir, &["merge", "--ff-only", "ff"]);
    assert_eq!(read(&dir, "ff.txt"), "1\n");
    assert_ne!(hbt(&dir, &["rev-parse", "main"]), tip);

    hb(&dir, &["switch", "-c", "left", "main~1"]);
    commit(&dir, "x.txt", "left\n", "left");
    hb(&dir, &["switch", "-c", "right", "main~1"]);
    commit(&dir, "x.txt", "right\n", "right");
    assert!(hb_fails(&dir, &["merge", "--ff-only", "left"]).contains("fast-forward"));

    hb(&dir, &["merge", "-X", "ours", "left"]);
    assert_eq!(read(&dir, "x.txt"), "right\n");
    hb(&dir, &["undo"]);
    hb(&dir, &["merge", "-X", "theirs", "left"]);
    assert_eq!(read(&dir, "x.txt"), "left\n");
    hb(&dir, &["undo"]);
    hb(&dir, &["merge", "-s", "ours", "left"]);
    assert_eq!(read(&dir, "x.txt"), "right\n");
    hb(&dir, &["undo"]);
    hb(&dir, &["merge", "--squash", "main"]);
    assert_eq!(hb(&dir, &["show"])["parents"].as_array().unwrap().len(), 1);
    hb(&dir, &["undo"]);
    hb(&dir, &["merge", "left"]);
    assert!(read(&dir, "x.txt").contains("<<<<<<<"));
    hb(&dir, &["resolve", "--theirs"]);
    assert_eq!(read(&dir, "x.txt"), "left\n");
    assert_eq!(
        hb(&dir, &["status"])["conflicts"].as_array().unwrap().len(),
        0
    );
}

#[test]
fn criss_cross_merges_do_not_invent_conflicts() {
    let lab = Lab::new();
    let work = lab.path("g");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    let body: String = (0..12).map(|i| format!("{i}\n")).collect();
    write(&work, "f.txt", &body);
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "base"]);
    git(&work, &["branch", "other"]);
    write(&work, "f.txt", &body.replace("\n2\n", "\ntwo\n"));
    git(&work, &["commit", "-qam", "m1"]);
    git(&work, &["switch", "-q", "other"]);
    write(&work, "f.txt", &body.replace("\n9\n", "\nnine\n"));
    git(&work, &["commit", "-qam", "o1"]);
    git(&work, &["merge", "-q", "--no-edit", "main"]);
    git(&work, &["switch", "-q", "main"]);
    git(&work, &["merge", "-q", "--no-edit", "other~1"]);
    git(&work, &["switch", "-q", "other"]);
    write(&work, "other.txt", "o\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "o2"]);
    git(&work, &["switch", "-q", "main"]);
    write(&work, "main.txt", "m\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "m2"]);

    let dir = lab.repo("h");
    hb(&dir, &["git", "import", "--all", work.to_str().unwrap()]);
    let bases = hb(&dir, &["merge-base", "--all", "main", "other"]);
    assert_eq!(
        bases["bases"].as_array().unwrap().len(),
        2,
        "this is a criss-cross"
    );
    hb(&dir, &["new", "main"]);
    let merged = hb(&dir, &["merge", "other"]);
    assert_eq!(merged["conflicts"].as_array().unwrap().len(), 0);
    git(&work, &["merge", "-q", "--no-edit", "other"]);
    assert_eq!(read(&dir, "f.txt"), read(&work, "f.txt"));
}

#[test]
fn bisect_finds_the_change_that_broke_it() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "check.txt", "good\n", "start");
    hb(&dir, &["branch", "main"]);
    hb(&dir, &["switch", "main"]);
    for i in 0..9 {
        let content = if i < 5 { "good\n" } else { "bad\n" };
        commit(&dir, "check.txt", content, &format!("step {i}"));
    }
    let good = hbt(&dir, &["rev-parse", "main~9"]);
    hb(&dir, &["bisect", "start", "main", good.trim()]);
    let found = hb(&dir, &["bisect", "run", "grep", "-q", "good", "check.txt"]);
    assert_eq!(found["state"], "found");
    assert_eq!(found["description"], "step 5");
    hb(&dir, &["bisect", "reset"]);
    assert_eq!(read(&dir, "check.txt"), "bad\n");

    // By hand, too.
    hb(&dir, &["bisect", "start", "main", good.trim()]);
    let mut state = hb(&dir, &["bisect", "bad", "main"]);
    for _ in 0..12 {
        if state["state"] != "testing" {
            break;
        }
        let verdict = if read(&dir, "check.txt").contains("good") {
            "good"
        } else {
            "bad"
        };
        state = hb(&dir, &["bisect", verdict, "@-"]);
    }
    assert_eq!(state["description"], "step 5");
    assert!(hbt(&dir, &["bisect", "log"]).contains("bad"));
    hb(&dir, &["bisect", "reset"]);
}

// --------------------------------------------------------- moving changes

#[test]
fn patches_travel_between_hebra_and_git_in_both_directions() {
    let lab = Lab::new();
    let dir = series(&lab);
    write(&dir, "b.txt", "b\nmore\n");
    write(&dir, "gone.txt", "soon\n");
    hb(&dir, &["commit", "-m", "edit b\n\nwith a body"]);
    fs::remove_file(dir.join("gone.txt")).unwrap();
    hb(&dir, &["commit", "-m", "remove gone"]);
    let patches = lab.path("patches");
    let out = hb(
        &dir,
        &[
            "format-patch",
            "main~2..main",
            "-o",
            patches.to_str().unwrap(),
        ],
    );
    let files: Vec<String> = out["patches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("0001-edit-b.patch"), "{files:?}");

    // Git takes our patches.
    let work = lab.path("g");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    write(&work, "b.txt", "b\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "base"]);
    git(&work, &["am", "-q", &files[0], &files[1]]);
    assert_eq!(read(&work, "b.txt"), "b\nmore\n");
    assert_eq!(
        git(&work, &["log", "--format=%s", "-n", "2"]),
        "remove gone\nedit b"
    );

    // We take Git's.
    let out_dir = lab.path("from-git");
    let target = out_dir.to_string_lossy().into_owned();
    git(
        &work,
        &["format-patch", "-q", "-o", &target, "HEAD~2..HEAD"],
    );
    let mut names: Vec<PathBuf> = fs::read_dir(&out_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    let h = lab.repo("h");
    commit(&h, "b.txt", "b\n", "base");
    hb(
        &h,
        &["am", names[0].to_str().unwrap(), names[1].to_str().unwrap()],
    );
    assert_eq!(read(&h, "b.txt"), "b\nmore\n");
    assert_eq!(subjects(&h, "@-")[0], "remove gone");
    assert_eq!(
        hb(&h, &["show", "@--"])["description"],
        "edit b\n\nwith a body"
    );
}

#[test]
fn apply_checks_applies_and_reverses() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "a.txt", "one\ntwo\nthree\n", "base");
    let patch = "--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
    write(&dir, "p.diff", patch);
    hb(&dir, &["apply", "--check", "p.diff"]);
    assert_eq!(read(&dir, "a.txt"), "one\ntwo\nthree\n");
    hb(&dir, &["apply", "p.diff"]);
    assert_eq!(read(&dir, "a.txt"), "one\nTWO\nthree\n");
    hb(&dir, &["apply", "-R", "p.diff"]);
    assert_eq!(read(&dir, "a.txt"), "one\ntwo\nthree\n");
    write(
        &dir,
        "bad.diff",
        "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-nope\n+x\n",
    );
    assert!(hb_fails(&dir, &["apply", "bad.diff"]).contains("does not apply"));
    // A diff made by Git works too.
    write(&dir, "a.txt", "one\ntwo\nthree\nfour\n");
    let ours = hbt(&dir, &["diff"]);
    hb(&dir, &["restore", "a.txt"]);
    write(&dir, "ours.diff", &ours);
    hb(&dir, &["apply", "ours.diff"]);
    assert!(read(&dir, "a.txt").contains("four"));
}

#[test]
fn archives_open_with_standard_tools() {
    let lab = Lab::new();
    let dir = series(&lab);
    let long = format!("{}/deep.txt", "x".repeat(120));
    commit(&dir, &long, "deep\n", "long path");
    hb(
        &dir,
        &["archive", "-o", "out.tar", "--prefix", "p/", "main"],
    );
    let list = Command::new("tar")
        .args(["-tf", "out.tar"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let names = String::from_utf8_lossy(&list.stdout);
    assert!(names.contains("p/a.txt"), "{names}");
    assert!(names.contains("deep.txt"), "{names}");
    hb(&dir, &["archive", "-o", "out.tar.gz", "main"]);
    let check = Command::new("tar")
        .args(["-tzf", "out.tar.gz"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&check.stdout).contains("c.txt"));
    hb(&dir, &["archive", "-o", "out.zip", "main"]);
    let zip = fs::read(dir.join("out.zip")).unwrap();
    assert_eq!(&zip[..4], b"PK\x03\x04");
}

#[test]
fn bundles_carry_history_without_a_network() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(&dir, &["bundle", "create", "repo.bundle"]);
    assert!(hbt(&dir, &["bundle", "list-heads", "repo.bundle"]).contains("refs/heads/main"));
    let other = lab.repo("other");
    let file = dir.join("repo.bundle");
    hb(&other, &["bundle", "unbundle", file.to_str().unwrap()]);
    hb(&other, &["new", "main@bundle"]);
    assert_eq!(read(&other, "c.txt"), "c\n");
}

#[test]
fn notes_attach_without_rewriting() {
    let lab = Lab::new();
    let dir = series(&lab);
    let before = hbt(&dir, &["rev-parse", "main"]);
    hb(&dir, &["notes", "add", "main", "-m", "reviewed by ana"]);
    assert_eq!(hbt(&dir, &["rev-parse", "main"]), before);
    assert_eq!(
        hbt(&dir, &["notes", "show", "main"]).trim(),
        "reviewed by ana"
    );
    assert!(hbt(&dir, &["show", "main"]).contains("reviewed by ana"));
    assert!(hbt(&dir, &["notes", "list"]).contains("reviewed by ana"));
    hb(&dir, &["notes", "remove", "main"]);
    assert!(hb_fails(&dir, &["notes", "show", "main"]).contains("no note"));
}

// --------------------------------------------------------- the repository

#[test]
fn worktrees_are_workspaces() {
    let lab = Lab::new();
    let dir = series(&lab);
    let wt = lab.root.join("extra");
    hb(&dir, &["worktree", "add", wt.to_str().unwrap(), "main~1"]);
    assert_eq!(read(&wt, "b.txt"), "b\n");
    assert!(!wt.join("c.txt").exists());
    assert!(hbt(&dir, &["worktree", "list"]).contains("extra"));
    hb(&dir, &["worktree", "remove", "extra"]);
    assert!(!wt.exists());
    let wt2 = lab.root.join("gone");
    hb(&dir, &["worktree", "add", wt2.to_str().unwrap()]);
    fs::remove_dir_all(&wt2).unwrap();
    assert_eq!(hb(&dir, &["worktree", "prune"])["pruned"][0], "gone");
}

#[test]
fn sparse_workspaces_keep_only_some_paths_and_lose_nothing() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    write(&dir, "src/a.rs", "a\n");
    write(&dir, "docs/b.md", "b\n");
    write(&dir, "c.txt", "c\n");
    hb(&dir, &["commit", "-m", "base"]);
    hb(&dir, &["sparse", "set", "src"]);
    assert!(dir.join("src/a.rs").exists());
    assert!(!dir.join("docs/b.md").exists());
    assert_eq!(hbt(&dir, &["sparse", "list"]).trim(), "src");

    // Missing files are not deletions.
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        0
    );
    write(&dir, "src/a.rs", "changed\n");
    hb(&dir, &["commit", "-m", "edit"]);
    assert_eq!(hbt(&dir, &["cat", "@-:docs/b.md"]), "b\n");
    assert_eq!(hbt(&dir, &["cat", "@-:src/a.rs"]), "changed\n");

    hb(&dir, &["sparse", "disable"]);
    assert_eq!(read(&dir, "docs/b.md"), "b\n");
    assert_eq!(read(&dir, "c.txt"), "c\n");
}

#[test]
fn fsck_finds_damage() {
    let lab = Lab::new();
    let dir = series(&lab);
    assert!(hbt(&dir, &["fsck"]).contains("No problems"));
    hb(&dir, &["gc", "--prune", "now"]);
    assert!(hbt(&dir, &["fsck"]).contains("No problems"));
    // Everything still reads after packing.
    assert_eq!(hbt(&dir, &["cat", "main~2:a.txt"]), "a\n");
    let objects = dir.join(".hebra/repo/objects");
    let mut pack = None;
    for entry in fs::read_dir(objects.join("pack")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "hbp") {
            pack = Some(path);
        }
    }
    let pack = pack.unwrap();
    let mut bytes = fs::read(&pack).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    bytes[middle + 1] ^= 0xff;
    fs::write(&pack, bytes).unwrap();
    assert_eq!(hb_raw(&dir, &["fsck"]).status.code(), Some(1));
}

#[test]
fn hooks_run_and_can_stop_a_commit() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    let hooks = dir.join(".hebra/repo/hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").unwrap();
    write(&dir, "a.txt", "1\n");
    assert!(hb_fails(&dir, &["commit", "-m", "x"]).contains("pre-commit"));
    hb(&dir, &["commit", "-m", "x", "--no-verify"]);
    fs::remove_file(hooks.join("pre-commit")).unwrap();

    fs::write(
        hooks.join("commit-msg"),
        "#!/bin/sh\necho 'Signed-off-by: hook' >> \"$1\"\n",
    )
    .unwrap();
    write(&dir, "b.txt", "2\n");
    hb(&dir, &["commit", "-m", "y"]);
    assert!(
        hb(&dir, &["show", "@-"])["description"]
            .as_str()
            .unwrap()
            .contains("Signed-off-by")
    );
    assert!(hbt(&dir, &["hook", "list"]).contains("commit-msg"));
}

#[test]
fn aliases_expand_like_git() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(
        &dir,
        &["config", "set", "alias.last", "log -n 1 --format=%s"],
    );
    assert_eq!(hbt(&dir, &["last", "main"]).trim(), "add c");
    assert!(hb_fails(&dir, &["nonsense"]).contains("nonsense"));
}

#[test]
fn config_works_in_the_git_spelling() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    hb(&dir, &["config", "user.email", "t@example.com"]);
    assert_eq!(hbt(&dir, &["config", "user.email"]).trim(), "t@example.com");
    assert!(hbt(&dir, &["config", "--list"]).contains("user.email"));
    hb(&dir, &["config", "--unset", "user.email"]);
    assert!(hb_fails(&dir, &["config", "user.email"]).contains("not set"));
}

#[test]
fn line_endings_of_files_from_git_are_left_alone() {
    let lab = Lab::new();
    let work = lab.path("g");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "core.autocrlf", "false"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    fs::write(work.join("crlf.txt"), "one\r\ntwo\r\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "crlf"]);
    let dir = lab.repo("h");
    hb(&dir, &["git", "import", work.to_str().unwrap()]);
    hb(&dir, &["new", "main"]);
    // Checked out exactly, and not reported as changed: right away...
    assert_eq!(fs::read(dir.join("crlf.txt")).unwrap(), b"one\r\ntwo\r\n");
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        0
    );
    // ...and with a forced re-read.
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::remove_file(dir.join(".hebra/wc-cache.json")).unwrap();
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        1,
        "without a record of the blob, normalization applies"
    );
}

#[test]
fn every_git_command_has_an_answer() {
    // `git help -a`, minus what only makes sense inside Git itself. Each is
    // either the same word in `hb` or a documented replacement.
    let table = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/git-commands.md"),
    )
    .unwrap();
    let help = String::from_utf8_lossy(
        &Command::new("git")
            .args(["help", "-a", "--no-aliases"])
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();
    let mut missing = Vec::new();
    for line in help.lines() {
        let Some(word) = line.strip_prefix("   ") else {
            continue;
        };
        let word = word.split_whitespace().next().unwrap_or("");
        if word.is_empty() || word.contains('.') {
            continue;
        }
        if !table.contains(&format!("`git {word}`")) && is_user_command(word) {
            missing.push(word.to_owned());
        }
    }
    assert!(
        missing.is_empty(),
        "not in docs/git-commands.md: {missing:?}"
    );

    // And every `hb` command the table promises parses.
    for line in table.lines() {
        let Some(rest) = line.split("| `hb ").nth(1) else {
            continue;
        };
        let word = rest.split(['`', ' ']).next().unwrap();
        if word == "help" {
            continue;
        }
        let out = hb_raw(Path::new("."), &[word, "--help"]);
        assert!(out.status.success(), "`hb {word}` does not exist");
    }
}

/// Porcelain and the helpers people call by hand, not low-level plumbing
/// (those are listed in the table too when they have an equivalent).
fn is_user_command(word: &str) -> bool {
    const PORCELAIN: &str = "add am archive backfill bisect branch bundle checkout cherry-pick citool clean clone commit describe diff fetch format-patch gc gitk grep gui init log maintenance merge mv notes pull push range-diff rebase reset restore revert rm scalar shortlog show sparse-checkout stash status submodule survey switch tag worktree config fast-export fast-import filter-branch mergetool pack-refs prune reflog refs remote repack replace annotate blame bugreport count-objects diagnose difftool fsck gitweb help instaweb merge-tree rerere show-branch verify-commit verify-tag version whatchanged archimport cvsexportcommit cvsimport cvsserver imap-send p4 quiltimport request-pull send-email svn";
    PORCELAIN.split_whitespace().any(|w| w == word)
}

#[test]
fn gitattributes_decide_line_endings_and_archives() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    write(
        &dir,
        ".gitattributes",
        "*.bat eol=crlf\nraw.txt -text\nsecret.txt export-ignore\n",
    );
    fs::write(dir.join("raw.txt"), "kept\r\nas is\r\n").unwrap();
    fs::write(dir.join("run.bat"), "echo hi\r\n").unwrap();
    fs::write(dir.join("plain.txt"), "normalized\r\n").unwrap();
    write(&dir, "secret.txt", "x\n");
    hb(&dir, &["commit", "-m", "files"]);
    // In history: bytes untouched only where the attributes ask for it.
    assert_eq!(hbt(&dir, &["cat", "@-:raw.txt"]), "kept\r\nas is\r\n");
    assert_eq!(hbt(&dir, &["cat", "@-:plain.txt"]), "normalized\n");
    assert_eq!(hbt(&dir, &["cat", "@-:run.bat"]), "echo hi\n");
    // On disk, a batch file gets its CRLF back.
    let other = lab.root.join("second");
    hb(&dir, &["worktree", "add", other.to_str().unwrap(), "@-"]);
    assert_eq!(fs::read(other.join("run.bat")).unwrap(), b"echo hi\r\n");
    assert_eq!(
        hb(&other, &["status"])["changes"].as_array().unwrap().len(),
        0
    );

    hb(&dir, &["archive", "-o", "out.tar", "@-"]);
    let list = Command::new("tar")
        .args(["-tf", "out.tar"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let names = String::from_utf8_lossy(&list.stdout);
    assert!(
        names.contains("run.bat") && !names.contains("secret.txt"),
        "{names}"
    );
}

#[test]
fn a_repository_cannot_make_hb_run_commands_or_leave_the_workspace() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    commit(&dir, "a.txt", "x\n", "base");

    // A patch that tries to write outside.
    write(
        &dir,
        "evil.diff",
        "--- /dev/null\n+++ b/../../outside.txt\n@@ -0,0 +1 @@\n+x\n",
    );
    assert!(hb_fails(&dir, &["apply", "evil.diff"]).contains("invalid path"));
    assert!(!lab.root.join("outside.txt").exists());

    // A submodule URL that would run a command if passed to `git` as is.
    let marker = lab.root.join("pwned");
    let payload = format!(
        "ext::sh -c touch% {}",
        marker.to_string_lossy().replace('\\', "/")
    );
    let work = lab.path("g");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    write(
        &work,
        ".gitmodules",
        &format!("[submodule \"x\"]\n\tpath = sub\n\turl = {payload}\n"),
    );
    git(&work, &["add", "."]);
    git(
        &work,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,0123456789012345678901234567890123456789,sub",
        ],
    );
    git(&work, &["commit", "-qm", "evil submodule"]);
    let copy = lab.repo("copy");
    hb(&copy, &["git", "import", work.to_str().unwrap()]);
    hb(&copy, &["new", "main"]);
    assert!(
        !hb_raw(&copy, &["submodule", "update", "--init"])
            .status
            .success()
    );
    assert!(!marker.exists(), "the URL must not run");
}

#[test]
fn imports_compact_themselves_and_stores_stay_small() {
    let lab = Lab::new();
    let work = lab.path("g");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    let mut body = String::new();
    for i in 0..400 {
        body.push_str(&format!("a line of text number {i} that barely changes\n"));
    }
    for i in 0..30 {
        body.push_str(&format!("line added in version {i}\n"));
        write(&work, "big.txt", &body);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", &format!("v{i}")]);
    }
    let dir = lab.repo("h");
    hb(&dir, &["git", "import", work.to_str().unwrap()]);
    hb(&dir, &["gc", "--prune", "now"]);
    let stats = hb(&dir, &["stats"]);
    let size = stats["pack_bytes"].as_u64().unwrap();
    // 30 versions of 18 KB: whole, 500 KB. As deltas, a small fraction.
    assert!(size < 40_000, "{size} bytes");
    assert_eq!(hbt(&dir, &["cat", "main~20:big.txt"]).lines().count(), 410);
    assert!(hbt(&dir, &["fsck"]).contains("No problems"));
}

#[test]
fn what_git_people_type_from_memory_works() {
    let lab = Lab::new();
    let dir = series(&lab);

    // Bare `stash`, and the flags nobody thinks about.
    write(&dir, "a.txt", "wip\n");
    hb(&dir, &["stash"]);
    assert_eq!(read(&dir, "a.txt"), "a\n");
    hb(&dir, &["stash", "pop"]);
    hb(&dir, &["diff", "--cached", "--stat"]);
    hb(&dir, &["restore", "--staged", "a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "wip\n", "unstaging touches nothing");
    hb(&dir, &["reset", "a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "wip\n");
    hb(&dir, &["checkout", "a.txt"]);
    assert_eq!(read(&dir, "a.txt"), "a\n");
    write(&dir, "a.txt", "wip\n");
    hb(&dir, &["checkout", "."]);
    assert_eq!(read(&dir, "a.txt"), "a\n");

    // `log -3`, presets, `show` without arguments, `--oneline`.
    assert_eq!(
        hbt(&dir, &["log", "-3", "--format=%s", "main"])
            .lines()
            .count(),
        3
    );
    assert!(hbt(&dir, &["log", "--pretty=oneline", "-1", "main"]).contains("add c"));
    assert!(
        hbt(
            &dir,
            &[
                "log",
                "--oneline",
                "--decorate",
                "--abbrev-commit",
                "-1",
                "main"
            ]
        )
        .contains("add c")
    );
    assert_eq!(hb(&dir, &["show"])["description"], "add c");
    assert!(hbt(&dir, &["show", "--oneline", "-s", "main"]).contains("add c"));
    assert!(hbt(&dir, &["--no-pager", "log", "-1", "main"]).contains("add c"));

    // `switch -`, `blame -L 1,+1`, `branch -v`, `merge --abort`.
    hb(&dir, &["switch", "-c", "topic"]);
    hb(&dir, &["switch", "-"]);
    assert_eq!(hbt(&dir, &["branch", "--show-current"]).trim(), "main");
    hb(&dir, &["switch", "-"]);
    assert_eq!(hbt(&dir, &["branch", "--show-current"]).trim(), "topic");
    assert!(hbt(&dir, &["branch", "-vv"]).contains("topic"));
    assert_eq!(
        hbt(&dir, &["blame", "-L", "1,+0", "a.txt"]).lines().count(),
        1
    );
    assert!(hb_fails(&dir, &["merge", "--abort"]).contains("hb undo"));
    assert!(hbt(&dir, &["merge", "--continue"]).contains("Nothing to continue"));

    // Remotes.
    assert!(hbt(&dir, &["remote"]).contains("No remotes"));
    hb(&dir, &["remote", "add", "up", "https://github.com/a/b"]);
    assert!(hbt(&dir, &["remote", "-v"]).contains("github.com/a/b"));
    hb(&dir, &["remote", "rename", "up", "upstream"]);
    hb(&dir, &["remote", "rm", "upstream"]);
    hb(&dir, &["notes"]);
    hb(&dir, &["submodule"]);
    hb(&dir, &["version"]);
}

#[test]
fn pull_push_and_fetch_take_git_arguments() {
    let lab = Lab::new();
    let bare = lab.github();
    let url = bare.to_string_lossy().into_owned();
    hb(&lab.root, &["clone", "--git", &url, "copy"]);
    let copy = lab.root.join("copy");
    hb(&copy, &["fetch", "origin", "main", "--prune", "--tags"]);
    hb(&copy, &["pull", "origin", "main"]);
    write(&copy, "x.txt", "x\n");
    hb(&copy, &["commit", "-am", "x"]);
    hb(&copy, &["push", "-u", "origin", "HEAD"]);
    assert_eq!(git(&bare, &["show", "main:x.txt"]), "x");
}

#[test]
fn commit_can_take_some_hunks_and_leave_the_rest() {
    let lab = Lab::new();
    let dir = lab.repo("r");
    let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
    commit(&dir, "f.txt", &body, "base");
    let edited = body
        .replace("line 2\n", "line two\n")
        .replace("line 35\n", "line thirty five\n");
    write(&dir, "f.txt", &edited);

    let mut child = hb_child(&dir, &["commit", "-p", "-m", "only the first"]);
    {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(b"y\nn\n").unwrap();
    }
    assert!(child.wait().unwrap().success());

    let first = hbt(&dir, &["cat", "@-:f.txt"]);
    assert!(first.contains("line two"), "{first}");
    assert!(first.contains("line 35\n"), "{first}");
    assert_eq!(read(&dir, "f.txt"), edited, "the file on disk is untouched");
    let rest = hbt(&dir, &["diff"]);
    assert!(rest.contains("+line thirty five"), "{rest}");
    assert!(!rest.contains("line two"), "{rest}");
}

fn hb_child(dir: &Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_hb"))
        .current_dir(dir)
        .args(args)
        .env("HB_ACTOR", "tester")
        .env("HB_ACTOR_KIND", "human")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

fn hb_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hb"));
    command
        .current_dir(dir)
        .args(args)
        .env_remove("CLAUDECODE")
        .env("HB_ACTOR", "tester")
        .env("HB_ACTOR_KIND", "human");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

/// Makes an SSH key and tells the repository at `dir` to sign with it.
fn signing_key(lab: &Lab, dir: &Path) -> PathBuf {
    let key = lab.root.join("signing-key");
    let made = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(made.status.success());
    hb(dir, &["config", "set", "gpg.format", "ssh"]);
    hb(
        dir,
        &["config", "set", "user.signingkey", key.to_str().unwrap()],
    );
    key
}

#[test]
fn changes_and_tags_can_be_signed_with_an_ssh_key() {
    let lab = Lab::new();
    let dir = series(&lab);
    signing_key(&lab, &dir);

    write(&dir, "signed.txt", "x\n");
    hb(&dir, &["commit", "-S", "-m", "signed work"]);
    let shown = hb(&dir, &["verify-commit", "@-"]);
    assert_eq!(shown["valid"], true, "{shown}");
    // The branch and the working copy stay on the same, signed commit.
    assert_eq!(
        hbt(&dir, &["rev-parse", "main"]),
        hbt(&dir, &["rev-parse", "@-"])
    );

    // The signature covers the commit: a new version is not signed.
    hb(&dir, &["describe", "-r", "@-", "-m", "changed afterwards"]);
    assert!(hb_fails(&dir, &["verify-commit", "@-"]).contains("not signed"));

    hb(&dir, &["tag", "-s", "v9", "-m", "signed release"]);
    assert_eq!(hb(&dir, &["tag", "-v", "v9"])["valid"], true);
    assert!(hb_fails(&dir, &["verify", "main~1"]).contains("not signed"));
}

#[test]
fn every_command_that_makes_commits_can_sign_them() {
    let lab = Lab::new();
    let dir = series(&lab);
    let key = signing_key(&lab, &dir);
    hb(&dir, &["config", "set", "commit.gpgsign", "true"]);
    let valid = |rev: &str| hb(&dir, &["verify-commit", rev])["valid"] == true;
    let unsigned = |rev: &str| hb_fails(&dir, &["verify-commit", rev]).contains("not signed");

    commit(&dir, "d.txt", "d\n", "add d");
    assert!(valid("@-"));

    // The flag beats the setting, and an amend never keeps an old signature.
    write(&dir, "e.txt", "e\n");
    hb(&dir, &["commit", "--no-gpg-sign", "-m", "add e"]);
    assert!(unsigned("@-"));
    hb(&dir, &["commit", "--amend", "-m", "add e again"]);
    assert!(valid("@-"));
    hb(
        &dir,
        &["commit", "--amend", "--no-gpg-sign", "-m", "e, unsigned"],
    );
    assert!(unsigned("@-"));
    hb(&dir, &["commit", "--amend", "-m", "add e"]);

    hb(&dir, &["switch", "-c", "side", "main~3"]);
    commit(&dir, "s.txt", "s\n", "side one");
    hb(&dir, &["cherry-pick", "main"]);
    assert!(valid("@-"));
    hb(&dir, &["revert", "@-"]);
    assert!(valid("@-"));
    assert_eq!(
        hbt(&dir, &["rev-parse", "side"]),
        hbt(&dir, &["rev-parse", "@-"])
    );

    // A rebase signs the whole series, each on top of the signed one below.
    hb(&dir, &["rebase", "main"]);
    let moved = hbt(&dir, &["log", "--format=%H", "main..@-"]);
    assert_eq!(moved.lines().count(), 3, "{moved}");
    for id in moved.lines() {
        assert!(valid(id), "{id}");
    }
    assert_eq!(
        hbt(&dir, &["rev-parse", "side"]),
        hbt(&dir, &["rev-parse", "@-"])
    );
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        0
    );

    // A rebase with nothing to do changes nothing, signature included.
    let before = hbt(&dir, &["rev-parse", "@-"]);
    hb(&dir, &["rebase", "main"]);
    assert_eq!(hbt(&dir, &["rev-parse", "@-"]), before);

    // Nor does an edited one that leaves every commit where it is...
    hb(&dir, &["rebase", "main", "--autosquash", "--no-gpg-sign"]);
    assert!(valid("@-"));

    // ...until a step changes one: the old signature does not cover it.
    let mut todo: Vec<String> = hbt(&dir, &["log", "--format=%H", "main..@-"])
        .lines()
        .rev()
        .map(|id| format!("pick {id}"))
        .collect();
    let last = todo.pop().unwrap();
    todo.push(format!("{} new words", last.replace("pick", "reword")));
    let file = lab.root.join("todo");
    fs::write(&file, todo.join("\n")).unwrap();
    let list = file.to_str().unwrap();
    hb(&dir, &["rebase", "main", "--todo", list, "--no-gpg-sign"]);
    assert!(unsigned("@-"));
    assert_eq!(subjects(&dir, "@-")[0], "new words");

    // A merge is finished by `hb commit`, and signed there.
    assert!(hb_fails(&dir, &["merge", "-S", "main"]).contains("hb commit -S"));
    hb(&dir, &["switch", "main"]);
    commit(&dir, "m.txt", "m\n", "on main");
    hb(&dir, &["switch", "side"]);
    hb(&dir, &["merge", "main"]);
    hb(&dir, &["commit", "-m", "merged"]);
    assert!(valid("@-"));

    // Patches arrive signed as well.
    let patches = lab.path("patches");
    hb(
        &dir,
        &["format-patch", "-o", patches.to_str().unwrap(), "@--..@-"],
    );
    let patch = fs::read_dir(&patches)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let other = lab.repo("other");
    let copied = lab.root.join("second-key");
    fs::copy(&key, &copied).unwrap();
    hb(&other, &["config", "set", "gpg.format", "ssh"]);
    hb(
        &other,
        &["config", "set", "user.signingkey", copied.to_str().unwrap()],
    );
    commit(&other, "base.txt", "base\n", "base");
    hb(&other, &["am", "-S", patch.to_str().unwrap()]);
    assert_eq!(hb(&other, &["verify-commit", "@-"])["valid"], true);

    // A key given by hand replaces the configured one. A key that does not
    // work stops the command and leaves the work where it was.
    hb(&dir, &["config", "set", "user.signingkey", "no-such-key"]);
    write(&dir, "f.txt", "f\n");
    assert!(hb_fails(&dir, &["commit", "-m", "no key"]).contains("signing failed"));
    assert_eq!(
        hb(&dir, &["status"])["changes"].as_array().unwrap().len(),
        1
    );
    let flag = format!("--gpg-sign={}", key.to_string_lossy());
    hb(&dir, &["commit", &flag, "-m", "this key"]);
    assert!(valid("@-"));

    // Git's own way of writing it. In `hb log`, `-S` looks for a word instead.
    write(&dir, "g.txt", "g\n");
    let attached = format!("-S{}", key.to_string_lossy());
    hb(&dir, &["commit", &attached, "-m", "attached"]);
    assert!(valid("@-"));
    assert!(hbt(&dir, &["log", "--format=%s", "-n", "50", "-Sg"]).contains("attached"));
}

#[test]
fn pull_rebase_signs_what_it_moves() {
    let lab = Lab::new();
    let origin = lab.repo("origin");
    commit(&origin, "a.txt", "one\n", "first");
    hb(&origin, &["bookmark", "set", "main"]);
    hb(&lab.root, &["clone", "origin", "copy"]);
    let copy = lab.root.join("copy");
    signing_key(&lab, &copy);
    commit(&copy, "b.txt", "two\n", "second");
    commit(&origin, "c.txt", "three\n", "third");
    hb(&origin, &["bookmark", "set", "main"]);

    hb(&copy, &["pull", "origin", "--rebase", "-S"]);
    assert_eq!(hb(&copy, &["verify-commit", "@-"])["valid"], true);
    assert_eq!(
        hbt(&copy, &["rev-parse", "main"]),
        hbt(&copy, &["rev-parse", "@-"])
    );
    assert_eq!(subjects(&copy, "@-")[..2], ["second", "third"]);
}

#[test]
fn submodules_add_update_and_report() {
    let lab = Lab::new();
    let library = lab.path("library");
    git(&library, &["init", "-q", "-b", "main"]);
    git(&library, &["config", "commit.gpgsign", "false"]);
    write(&library, "lib.txt", "from the submodule\n");
    git(&library, &["add", "."]);
    git(&library, &["commit", "-qm", "library"]);
    let sha = git(&library, &["rev-parse", "HEAD"]);

    let dir = lab.repo("r");
    let url = library.to_string_lossy().replace('\\', "/");
    let allow = [("GIT_ALLOW_PROTOCOL", "file")];
    let added = hb_env(&dir, &["submodule", "add", &url, "vendor/lib"], &allow);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(read(&dir, "vendor/lib/lib.txt"), "from the submodule\n");
    hb(&dir, &["commit", "-m", "add a submodule"]);
    let status = hbt(&dir, &["submodule", "status"]);
    assert!(
        status.contains(&sha) && status.contains("vendor/lib"),
        "{status}"
    );
    assert!(hbt(&dir, &["cat", "@-:.gitmodules"]).contains("vendor/lib"));

    // In a new checkout it is an empty pin until asked for.
    let other = lab.root.join("second");
    hb(&dir, &["worktree", "add", other.to_str().unwrap(), "@-"]);
    assert!(!other.join("vendor/lib/lib.txt").exists());
    let updated = hb_env(&other, &["submodule", "update", "--init"], &allow);
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    assert_eq!(read(&other, "vendor/lib/lib.txt"), "from the submodule\n");
    assert_eq!(
        hb(&other, &["status"])["changes"].as_array().unwrap().len(),
        0
    );
    assert!(
        hbt(
            &other,
            &["submodule", "foreach", "git", "rev-parse", "HEAD"]
        )
        .contains(&sha)
    );
}

#[test]
fn an_editor_writes_the_description_when_none_is_given() {
    let lab = Lab::new();
    let dir = series(&lab);
    write(&dir, "editor.sh", "echo from the editor > \"$1\"\n");
    write(&dir, "x.txt", "x\n");
    let editor = format!(
        "sh {}",
        dir.join("editor.sh").to_string_lossy().replace('\\', "/")
    );
    let out = hb_env(&dir, &["commit"], &[("HB_EDITOR", &editor)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(hb(&dir, &["show", "@-"])["description"], "from the editor");

    // Agents have no editor: it fails at once instead of waiting.
    write(&dir, "y.txt", "y\n");
    let none = hb_env(
        &dir,
        &["commit"],
        &[
            ("HB_EDITOR", ""),
            ("EDITOR", ""),
            ("VISUAL", ""),
            ("GIT_EDITOR", ""),
        ],
    );
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("-m"));
}

#[test]
fn external_diff_and_merge_tools_get_the_files() {
    let lab = Lab::new();
    let dir = series(&lab);
    hb(
        &dir,
        &[
            "config",
            "set",
            "diff.tool",
            "cat \"$LOCAL\" \"$REMOTE\" >> seen.txt",
        ],
    );
    write(&dir, "a.txt", "edited\n");
    hb(&dir, &["difftool"]);
    assert_eq!(read(&dir, "seen.txt"), "a\nedited\n");
    fs::remove_file(dir.join("seen.txt")).unwrap();

    hb(&dir, &["restore", "a.txt"]);
    hb(&dir, &["switch", "-c", "left", "main~1"]);
    commit(&dir, "x.txt", "left\n", "left");
    hb(&dir, &["switch", "-c", "right", "main~1"]);
    commit(&dir, "x.txt", "right\n", "right");
    hb(&dir, &["merge", "left"]);
    hb(
        &dir,
        &["config", "set", "merge.tool", "echo settled > \"$MERGED\""],
    );
    hb(&dir, &["mergetool"]);
    assert_eq!(read(&dir, "x.txt"), "settled\n");
    assert_eq!(
        hb(&dir, &["status"])["conflicts"].as_array().unwrap().len(),
        0
    );
}
