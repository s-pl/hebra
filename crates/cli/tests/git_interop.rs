//! Hebra checked against Git itself: histories imported from Git and
//! exported back must give Git the exact same trees, and `hb status` must
//! see the same changes as `git status` on the same directory.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn hb(cwd: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_hb"))
        .current_dir(cwd)
        .env_remove("CLAUDECODE")
        .env_remove("HB_ACTOR")
        .env_remove("HB_ACTOR_KIND")
        .env_remove("HB_JSON")
        .env("USER", "tester")
        .env("USERNAME", "tester")
        .arg("--json")
        .args(args)
        .output()
        .expect("failed to run hb");
    assert!(
        out.status.success(),
        "hb {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is not JSON")
}

fn git_command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .env("GIT_AUTHOR_NAME", "Ana")
        .env("GIT_AUTHOR_EMAIL", "ana@example.com")
        .env("GIT_COMMITTER_NAME", "Ana")
        .env("GIT_COMMITTER_EMAIL", "ana@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        );
    cmd
}

fn git_bytes(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Vec<u8> {
    let mut child = git_command(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git not found");
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn git(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(dir, args, None))
        .unwrap()
        .trim()
        .to_owned()
}

fn git_init(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["config", "core.symlinks", "false"]);
    // Modes come from `update-index --chmod`, so later `add -A` keep them.
    git(dir, &["config", "core.filemode", "false"]);
}

fn write(root: &Path, path: &str, content: &[u8]) {
    let full = root.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Adds a symlink to the index without needing symlink support on disk.
fn stage_symlink(dir: &Path, path: &str, target: &str) {
    let blob = String::from_utf8(git_bytes(
        dir,
        &["hash-object", "-w", "--stdin"],
        Some(target.as_bytes()),
    ))
    .unwrap();
    let info = format!("120000,{},{path}", blob.trim());
    git(dir, &["update-index", "--add", "--cacheinfo", &info]);
    // What Git itself writes with core.symlinks=false, so `add -A` keeps it.
    write(dir, path, target.as_bytes());
}

fn trees(dir: &Path, rev: &str) -> Vec<String> {
    let mut trees: Vec<String> = git(dir, &["log", "--format=%T", rev])
        .lines()
        .map(str::to_owned)
        .collect();
    trees.sort();
    trees
}

/// A Git history with the things that usually break converters: binary
/// files, executables, symlinks, spaces and non-ASCII names, deep
/// directories, a merge, deletions, renames, and a file that becomes a
/// directory (Git deletes `x` after adding `x/...`).
fn tricky_history(dir: &Path) {
    git_init(dir);
    write(dir, "README.md", b"# demo\n");
    write(dir, "src/lib.rs", b"pub fn one() -> u8 {\n    1\n}\n");
    write(dir, "dir with space/ñandú.txt", b"hola\n");
    write(dir, "bin.dat", b"\x00\x01\x02binary\xff");
    write(dir, "tools", b"a file for now\n");
    commit_all(dir, "first");

    write(
        dir,
        "src/lib.rs",
        b"pub fn one() -> u8 {\n    1\n}\n\npub fn two() {}\n",
    );
    write(dir, "run.sh", b"#!/bin/sh\necho hi\n");
    write(dir, "a/b/c/d/e.txt", b"deep\n");
    git(dir, &["add", "-A"]);
    git(dir, &["update-index", "--chmod=+x", "run.sh"]);
    stage_symlink(dir, "latest", "src/lib.rs");
    git(dir, &["commit", "-q", "-m", "second: exec, symlink, deep"]);

    git(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "README.md", b"# demo\n\nWith a feature.\n");
    commit_all(dir, "feature work");
    git(dir, &["checkout", "-q", "main"]);
    write(
        dir,
        "src/lib.rs",
        b"pub fn one() -> u8 {\n    1\n}\n\npub fn two() {}\n\npub fn three() {}\n",
    );
    commit_all(dir, "main work");
    git(
        dir,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );

    git(dir, &["rm", "-q", "-r", "dir with space"]);
    fs::create_dir_all(dir.join("assets")).unwrap();
    git(dir, &["mv", "bin.dat", "assets/bin.dat"]);
    commit_all(dir, "delete and rename");

    fs::remove_file(dir.join("tools")).unwrap();
    write(dir, "tools/tools", b"now a folder\n");
    write(dir, "tools/other", b"with more\n");
    commit_all(dir, "file becomes a folder");
}

#[test]
fn git_history_round_trips_with_identical_trees() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    tricky_history(&source);

    let repo = tmp.path().join("hb");
    fs::create_dir_all(&repo).unwrap();
    hb(&repo, &["init"]);
    let imported = hb(&repo, &["git", "import", source.to_str().unwrap()]);
    assert_eq!(imported["stats"]["commits"], 7);
    assert_eq!(imported["stats"]["skipped_entries"], 0);

    let out = tmp.path().join("out.git");
    hb(
        &repo,
        &["git", "export", out.to_str().unwrap(), "--revision", "main"],
    );

    // Every commit has the same tree, byte for byte, as in the original.
    assert_eq!(trees(&out, "main"), trees(&source, "main"));
    assert_eq!(
        git(&out, &["rev-parse", "main^{tree}"]),
        git(&source, &["rev-parse", "main^{tree}"])
    );
    assert_eq!(git(&out, &["rev-list", "--merges", "--count", "main"]), "1");
    git(&out, &["fsck", "--strict", "--no-progress"]);

    // The history checks out with the symlink and the executable bit.
    hb(&repo, &["new", "main"]);
    let link = repo.join("latest");
    let meta = fs::symlink_metadata(&link).unwrap();
    if meta.file_type().is_symlink() {
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("src/lib.rs"));
    } else {
        // No symlink support (Windows without the privilege): like Git with
        // core.symlinks=false, the file holds the target.
        assert_eq!(fs::read(&link).unwrap(), b"src/lib.rs");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(repo.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0);
    }
    // Checking out must not look like an edit, symlink included.
    let status = hb(&repo, &["status"]);
    assert_eq!(status["changes"], Value::Array(Vec::new()), "{status}");
}

/// A tiny deterministic generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }
}

fn git_changed(dir: &Path) -> BTreeSet<String> {
    let raw = git_bytes(
        dir,
        &["status", "--porcelain=v1", "-z", "-uall", "--no-renames"],
        None,
    );
    raw.split(|&b| b == 0)
        .filter(|entry| entry.len() > 3)
        .map(|entry| String::from_utf8(entry[3..].to_vec()).unwrap())
        .collect()
}

fn hb_changed(dir: &Path) -> BTreeSet<String> {
    hb(dir, &["status"])["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_owned())
        .collect()
}

fn tracked(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if name == ".git" || name == ".hebra" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            tracked(root, &entry.path(), out);
        } else {
            let rel = entry.path().strip_prefix(root).unwrap().to_owned();
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn status_agrees_with_git_on_random_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("both");
    git_init(&dir);
    for i in 0..12 {
        write(
            &dir,
            &format!("src/m{i}.rs"),
            format!("// {i}\n").as_bytes(),
        );
    }
    commit_all(&dir, "base");
    hb(&dir, &["init"]);
    hb(&dir, &["commit", "-m", "base"]);

    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..8 {
        for step in 0..6 {
            let mut files = Vec::new();
            tracked(&dir, &dir, &mut files);
            files.sort();
            match rng.below(5) {
                0 if !files.is_empty() => {
                    let path = &files[rng.below(files.len())];
                    fs::remove_file(dir.join(path)).unwrap();
                }
                1 => {
                    let depth = rng.below(3);
                    let mut path = String::new();
                    for d in 0..depth {
                        path.push_str(&format!("d{}/", (round + d) % 3));
                    }
                    path.push_str(&format!("new-{round}-{step}.txt"));
                    write(&dir, &path, format!("{round}.{step}\n").as_bytes());
                }
                2 if !files.is_empty() => {
                    let path = &files[rng.below(files.len())];
                    write(&dir, path, &[0, 1, 2, u8::try_from(step).unwrap()]);
                }
                _ if !files.is_empty() => {
                    let path = files[rng.below(files.len())].clone();
                    let mut data = fs::read(dir.join(&path)).unwrap();
                    data.extend_from_slice(format!("edit {round}.{step}\n").as_bytes());
                    write(&dir, &path, &data);
                }
                _ => {}
            }
        }
        assert_eq!(hb_changed(&dir), git_changed(&dir), "round {round}");
        commit_all(&dir, &format!("round {round}"));
        hb(&dir, &["commit", "-m", &format!("round {round}")]);
    }

    // After all that, Hebra's last commit is the very tree Git has.
    let out = tmp.path().join("out.git");
    hb(&dir, &["git", "export", out.to_str().unwrap()]);
    assert_eq!(
        git(&out, &["rev-parse", "main^{tree}"]),
        git(&dir, &["rev-parse", "HEAD^{tree}"])
    );
}
