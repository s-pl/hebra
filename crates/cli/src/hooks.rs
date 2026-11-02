//! Repository hooks, like Git's: executable files named after the event, in
//! `.hebra/hooks` (or wherever `core.hooksPath` says). They never come with a
//! clone, so a repository cannot make you run its code.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use hebra_core::Workspace;
use hebra_core::wc::DOT_DIR;
use serde_json::json;

use crate::commands::Output;

/// A command line for the shell. On Windows that is the `sh` every Git
/// installation brings, so hooks and scripts are portable; it is rarely on
/// the `PATH` outside Git Bash, so it is found through `git` itself.
pub fn shell(command: &str, cwd: &Path) -> Command {
    let mut process = match windows_sh() {
        Some(sh) => {
            let mut p = Command::new(sh);
            p.args(["-c", command]);
            p
        }
        None if cfg!(windows) => {
            let mut p = Command::new("cmd");
            p.args(["/C", command]);
            p
        }
        None => {
            let mut p = Command::new("sh");
            p.args(["-c", command]);
            p
        }
    };
    process.current_dir(cwd);
    process
}

/// The `sh` to use on Windows; `None` elsewhere, or without one.
fn windows_sh() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let works = |sh: &Path| {
        Command::new(sh)
            .args(["-c", "exit 0"])
            .status()
            .is_ok_and(|s| s.success())
    };
    let plain = PathBuf::from("sh");
    if works(&plain) {
        return Some(plain);
    }
    let out = Command::new("git").arg("--exec-path").output().ok()?;
    let exec = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    // `.../mingw64/libexec/git-core` -> `.../bin/sh.exe`.
    let root = exec.ancestors().nth(3)?;
    [
        root.join("bin").join("sh.exe"),
        root.join("usr").join("bin").join("sh.exe"),
    ]
    .into_iter()
    .find(|sh| sh.is_file())
}

fn directory(ws: &Workspace) -> PathBuf {
    ws.config_get("core.hookspath")
        .ok()
        .flatten()
        .or_else(|| ws.config_get("core.hooksPath").ok().flatten())
        .map_or_else(|| ws.repo_dir.join("hooks"), PathBuf::from)
}

/// Like Git, a file without the execute bit is not a hook, so it is skipped and
/// never stops a command. Windows has no such bit.
fn runnable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn find(ws: &Workspace, name: &str) -> Option<PathBuf> {
    let path = directory(ws).join(name);
    runnable(&path).then_some(path)
}

fn run(ws: &Workspace, path: &Path, args: &[&str]) -> Result<std::process::ExitStatus> {
    let mut line = format!("\"{}\"", path.display());
    for arg in args {
        line.push_str(&format!(" \"{arg}\""));
    }
    shell(&line, &ws.root)
        .env("HB_REPO", &ws.repo_dir)
        .status()
        .with_context(|| format!("running the hook {}", path.display()))
}

/// Runs a hook that can stop the command.
pub fn run_required(ws: &Workspace, name: &str, args: &[&str]) -> Result<()> {
    if let Some(path) = find(ws, name) {
        let status = run(ws, &path, args)?;
        if !status.success() {
            bail!("the `{name}` hook failed ({status}); use --no-verify to skip it");
        }
    }
    Ok(())
}

/// Runs a hook that only observes.
pub fn run_optional(ws: &Workspace, name: &str, args: &[&str]) {
    if let Some(path) = find(ws, name) {
        let _ = run(ws, &path, args);
    }
}

/// The `commit-msg` hook can change the description.
pub fn commit_msg(ws: &Workspace, text: &str) -> Result<String> {
    let Some(path) = find(ws, "commit-msg") else {
        return Ok(text.to_owned());
    };
    let file = ws.root.join(DOT_DIR).join("COMMIT_MSG");
    std::fs::write(&file, text)?;
    let status = run(ws, &path, &[&file.display().to_string()])?;
    if !status.success() {
        let _ = std::fs::remove_file(&file);
        bail!("the `commit-msg` hook failed ({status}); use --no-verify to skip it");
    }
    let edited = std::fs::read_to_string(&file)?;
    let _ = std::fs::remove_file(&file);
    Ok(edited.trim_end().to_owned())
}

pub fn list(ws: &Workspace) -> Output {
    let dir = directory(ws);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| runnable(&e.path()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let mut text = format!("Hooks in {}:\n", dir.display());
    for name in &names {
        text.push_str(&format!("  {name}\n"));
    }
    if names.is_empty() {
        text.push_str("  (none)\n");
    }
    Output::new(
        json!({ "directory": dir.display().to_string(), "hooks": names }),
        text,
    )
}

pub fn run_by_hand(ws: &Workspace, name: &str, args: &[String]) -> Result<Output> {
    let Some(path) = find(ws, name) else {
        bail!("no `{name}` hook in {}", directory(ws).display());
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = run(ws, &path, &args)?;
    Ok(Output::with_exit(
        json!({ "hook": name, "success": status.success() }),
        String::new(),
        u8::from(!status.success()),
    ))
}
