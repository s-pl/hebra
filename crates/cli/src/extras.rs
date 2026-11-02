//! The rest of what people use Git for: bisect, submodules, workspaces as
//! worktrees, integrity checks, external diff and merge tools.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::model::{Actor, Provenance};
use hebra_core::store::BLOB;
use hebra_core::wc::{self, DOT_DIR};
use hebra_core::{Error, ObjectId, Tx, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::{BisectCommand, SparseCommand, SubmoduleCommand};
use crate::commands::{Output, short_line};
use crate::hooks;
use crate::porcelain::Coded;

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidArgument(message.into())
}

// ---------------------------------------------------------------- bisect

#[derive(Debug, Default, Serialize, Deserialize)]
struct Bisect {
    /// Where to go back to.
    start: Option<ObjectId>,
    bad: Option<ObjectId>,
    good: Vec<ObjectId>,
    skipped: Vec<ObjectId>,
    log: Vec<String>,
}

fn bisect_path(ws: &Workspace) -> PathBuf {
    ws.root.join(DOT_DIR).join("bisect.json")
}

fn bisect_load(ws: &Workspace) -> Option<Bisect> {
    let raw = std::fs::read(bisect_path(ws)).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn bisect_save(ws: &Workspace, state: &Bisect) -> Result<()> {
    std::fs::write(bisect_path(ws), serde_json::to_vec(state)?)?;
    Ok(())
}

/// The next revision to test, or the answer.
enum Next {
    Test(ObjectId, usize),
    Found(ObjectId),
    Stuck(Vec<ObjectId>),
    NeedMarks,
}

fn bisect_next(tx: &mut Tx<'_>, state: &Bisect) -> Coded<Next> {
    let Some(bad) = state.bad else {
        return Ok(Next::NeedMarks);
    };
    if state.good.is_empty() {
        return Ok(Next::NeedMarks);
    }
    let mut excluded = std::collections::HashSet::new();
    for good in &state.good {
        excluded.extend(tx.ancestors(*good)?);
    }
    let mut suspects = Vec::new();
    for id in tx.walk_from(&[bad], usize::MAX)? {
        if !excluded.contains(&id) {
            suspects.push(id);
        }
    }
    suspects.reverse();
    let testable: Vec<ObjectId> = suspects
        .iter()
        .copied()
        .filter(|id| *id != bad && !state.skipped.contains(id))
        .collect();
    if testable.is_empty() {
        let unknown: Vec<ObjectId> = suspects
            .iter()
            .copied()
            .filter(|id| state.skipped.contains(id))
            .collect();
        return Ok(if unknown.is_empty() {
            Next::Found(bad)
        } else {
            let mut all = unknown;
            all.push(bad);
            Next::Stuck(all)
        });
    }
    Ok(Next::Test(testable[testable.len() / 2], testable.len()))
}

fn bisect_goto(tx: &mut Tx<'_>, id: ObjectId) -> Coded<()> {
    let tree = tx.commit(&id)?.tree;
    let wc = tx.new_commit(vec![id], tree, String::new(), Vec::new())?;
    tx.switch_wc(wc)?;
    tx.set_current_bookmark(None);
    Ok(())
}

fn bisect_step(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    state: &Bisect,
) -> Result<(Output, Option<ObjectId>)> {
    ws.run(actor, prov, "bisect", |tx| {
        match bisect_next(tx, state)? {
            Next::NeedMarks => Ok((
                Output::new(
                    json!({ "state": "waiting" }),
                    "Waiting for both a good and a bad revision.\n".to_owned(),
                ),
                None,
            )),
            Next::Found(id) => {
                let c = tx.commit(&id)?;
                Ok((
                    Output::new(
                        json!({ "state": "found", "commit": id.hex(), "description": c.description, "author": c.author, "provenance": c.provenance }),
                        format!("The first bad change is {}\n", short_line(&id, &c)),
                    ),
                    None,
                ))
            }
            Next::Stuck(ids) => {
                let list: Vec<String> = ids.iter().map(ObjectId::short).collect();
                Ok((
                    Output::new(
                        json!({ "state": "ambiguous", "candidates": ids.iter().map(ObjectId::hex).collect::<Vec<_>>() }),
                        format!(
                            "These could not be told apart: {}\n",
                            list.join(", ")
                        ),
                    ),
                    None,
                ))
            }
            Next::Test(id, left) => {
                bisect_goto(tx, id)?;
                let c = tx.commit(&id)?;
                let steps = usize::BITS - left.leading_zeros();
                Ok((
                    Output::new(
                        json!({ "state": "testing", "commit": id.hex(), "remaining": left }),
                        format!(
                            "Testing {} ({left} left, about {steps} step(s))\n",
                            short_line(&id, &c)
                        ),
                    ),
                    Some(id),
                ))
            }
        }
    })
    .map_err(Into::into)
}

fn mark(tx: &mut Tx<'_>, revision: Option<&str>) -> Coded<ObjectId> {
    match revision {
        Some(rev) => tx.resolve(rev),
        None => tx.resolve("@-"),
    }
}

pub fn bisect(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    cmd: &BisectCommand,
) -> Result<Output> {
    if let BisectCommand::Start { bad, good } = cmd {
        let (bad, good, start) = ws.look(actor, prov, |tx| {
            let start = tx.resolve("@-")?;
            let bad = bad.as_deref().map(|r| tx.resolve(r)).transpose()?;
            let good = good.as_deref().map(|r| tx.resolve(r)).transpose()?;
            Ok((bad, good, start))
        })?;
        let state = Bisect {
            start: Some(start),
            bad,
            good: good.into_iter().collect(),
            ..Bisect::default()
        };
        bisect_save(ws, &state)?;
        return Ok(bisect_step(ws, actor, prov, &state)?.0);
    }
    let mut state =
        bisect_load(ws).ok_or_else(|| anyhow!("no bisect running: `hb bisect start`"))?;
    match cmd {
        BisectCommand::Bad { revision } => {
            let id = ws.look(actor, prov, |tx| mark(tx, revision.as_deref()))?;
            state.log.push(format!("bad {id}"));
            state.bad = Some(id);
        }
        BisectCommand::Good { revision } => {
            let id = ws.look(actor, prov, |tx| mark(tx, revision.as_deref()))?;
            state.log.push(format!("good {id}"));
            state.good.push(id);
        }
        BisectCommand::Skip { revision } => {
            let id = ws.look(actor, prov, |tx| mark(tx, revision.as_deref()))?;
            state.log.push(format!("skip {id}"));
            state.skipped.push(id);
        }
        BisectCommand::Log => {
            let text: String = state.log.iter().map(|l| format!("{l}\n")).collect();
            return Ok(Output::new(json!({ "log": state.log }), text));
        }
        BisectCommand::Reset => {
            let start = state.start;
            std::fs::remove_file(bisect_path(ws))?;
            if let Some(start) = start {
                ws.run(actor, prov, "bisect", |tx| bisect_goto(tx, start))?;
            }
            return Ok(Output::new(
                json!({ "reset": true }),
                "Bisect finished.\n".to_owned(),
            ));
        }
        BisectCommand::Run { command } => return bisect_run(ws, actor, prov, state, command),
        BisectCommand::Start { .. } => unreachable!("handled above"),
    }
    bisect_save(ws, &state)?;
    Ok(bisect_step(ws, actor, prov, &state)?.0)
}

fn bisect_run(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    mut state: Bisect,
    command: &[String],
) -> Result<Output> {
    let line = command.join(" ");
    let (mut out, mut testing) = bisect_step(ws, actor, prov, &state)?;
    while let Some(id) = testing {
        let status = hooks::shell(&line, &ws.root)
            .status()
            .with_context(|| format!("running {line}"))?;
        match status.code() {
            Some(0) => {
                state.log.push(format!("good {id}"));
                state.good.push(id);
            }
            Some(125) => {
                state.log.push(format!("skip {id}"));
                state.skipped.push(id);
            }
            Some(128..) | None => bail!("the test command failed to run ({status})"),
            Some(_) => {
                state.log.push(format!("bad {id}"));
                state.bad = Some(id);
            }
        }
        bisect_save(ws, &state)?;
        (out, testing) = bisect_step(ws, actor, prov, &state)?;
    }
    Ok(out)
}

// ---------------------------------------------------------------- sparse

pub fn sparse(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    cmd: &SparseCommand,
) -> Result<Output> {
    ws.snapshot(actor, prov)?;
    let current = ws.sparse_paths();
    let wanted = match cmd {
        SparseCommand::List => {
            let text: String = current.iter().map(|p| format!("{p}\n")).collect();
            return Ok(Output::new(json!({ "paths": current }), text));
        }
        SparseCommand::Set { paths } => paths.clone(),
        SparseCommand::Add { paths } => {
            let mut all = current;
            all.extend(paths.iter().cloned());
            all
        }
        SparseCommand::Disable => Vec::new(),
    };
    ws.set_sparse(wanted.clone())?;
    Ok(Output::new(
        json!({ "paths": wanted }),
        if wanted.is_empty() {
            "All files are back.\n".to_owned()
        } else {
            format!("Keeping {}\n", wanted.join(", "))
        },
    ))
}

// ------------------------------------------------------------- worktrees

pub fn worktree_add(tx: &mut Tx<'_>, path: &Path, revision: &str) -> Coded<Output> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| invalid("that path has no name"))?;
    let base = tx.resolve(revision)?;
    let info = tx.create_workspace(&name, path, base, None, None)?;
    Ok(Output::new(
        json!({ "workspace": name, "path": info.path }),
        format!("Added workspace {name} at {}\n", info.path),
    ))
}

pub fn worktree_remove(tx: &mut Tx<'_>, name: &str, keep: bool) -> Coded<Output> {
    if name == tx.ws_name {
        return Err(invalid("cannot remove the current workspace"));
    }
    let info = tx
        .view
        .workspaces
        .remove(name)
        .ok_or_else(|| Error::WorkspaceNotFound(name.to_owned()))?;
    if !tx.is_empty_change(&info.wc)? {
        tx.view.heads.insert(info.wc);
    }
    if !keep {
        let path = PathBuf::from(&info.path);
        if path.join(DOT_DIR).join("workspace").is_file() {
            std::fs::remove_dir_all(&path).map_err(|source| Error::Io { path, source })?;
        }
    }
    Ok(Output::new(
        json!({ "removed": name }),
        format!("Removed workspace {name}\n"),
    ))
}

pub fn worktree_prune(tx: &mut Tx<'_>) -> Coded<Output> {
    let gone: Vec<String> = tx
        .view
        .workspaces
        .iter()
        .filter(|(name, w)| {
            name.as_str() != tx.ws_name
                && !Path::new(&w.path).join(DOT_DIR).join("workspace").is_file()
        })
        .map(|(name, _)| name.clone())
        .collect();
    for name in &gone {
        if let Some(info) = tx.view.workspaces.remove(name)
            && !tx.is_empty_change(&info.wc)?
        {
            tx.view.heads.insert(info.wc);
        }
    }
    Ok(Output::new(
        json!({ "pruned": gone }),
        format!("Forgot {} workspace(s).\n", gone.len()),
    ))
}

pub fn worktree_list(tx: &Tx<'_>) -> Output {
    let mut text = String::new();
    let mut rows = Vec::new();
    for (name, info) in &tx.view.workspaces {
        let _ = writeln!(text, "{}  {name}  {}", info.path, info.wc.short());
        rows.push(json!({ "name": name, "path": info.path, "commit": info.wc.hex() }));
    }
    Output::new(json!({ "worktrees": rows }), text)
}

// ------------------------------------------------------------ submodules

/// `path -> url`, from `.gitmodules`.
fn gitmodules(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(root.join(".gitmodules")) else {
        return out;
    };
    let (mut path, mut url) = (None, None);
    let mut flush = |path: &mut Option<String>, url: &mut Option<String>| {
        if let (Some(p), Some(u)) = (path.take(), url.take()) {
            out.insert(p, u);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            flush(&mut path, &mut url);
        } else if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "path" => path = Some(value.trim().to_owned()),
                "url" => url = Some(value.trim().to_owned()),
                _ => {}
            }
        }
    }
    flush(&mut path, &mut url);
    out
}

fn pinned(ws: &Workspace) -> Result<BTreeMap<String, String>> {
    let view = ws.view()?;
    let info = view
        .workspaces
        .get(&ws.name)
        .ok_or_else(|| anyhow!("workspace not found"))?;
    let commit: hebra_core::model::Commit = ws.store.read_json("commit", &info.wc)?;
    let tree = ws.store.read_tree(&commit.tree)?;
    let mut out = BTreeMap::new();
    for (path, entry) in tree.files {
        if entry.gitlink {
            let sha = ws.store.read(BLOB, &entry.blob)?;
            let sha = String::from_utf8_lossy(&sha).into_owned();
            // Both come from the repository: never let them act as options.
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("submodule {path} does not point at a commit id");
            }
            wc::safe_join(&ws.root, &path)?;
            out.insert(path, sha);
        }
    }
    Ok(out)
}

/// Runs git for something a repository's contents asked for, so it gets the
/// same limits Git puts on submodules: without this a URL like
/// `ext::sh -c ...` in `.gitmodules` would run commands.
fn git_at(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!("git: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn submodule_update(ws: &Workspace, init: bool) -> Result<Output> {
    let urls = gitmodules(&ws.root);
    let mut text = String::new();
    let mut rows = Vec::new();
    for (path, sha) in pinned(ws)? {
        let full = ws.root.join(&path);
        let populated = full.join(".git").exists();
        if !populated {
            if !init {
                let _ = writeln!(text, "{path}: not initialized (use --init)");
                continue;
            }
            let url = urls
                .get(&path)
                .ok_or_else(|| anyhow!("{path} is not in .gitmodules"))?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let _ = std::fs::remove_dir(&full);
            if url.starts_with('-') {
                bail!("{path} has an invalid URL in .gitmodules");
            }
            let target = full.display().to_string();
            git_at(&ws.root, &["clone", "--quiet", "--", url, &target])?;
        }
        git_at(&full, &["fetch", "--quiet", "origin", &sha]).ok();
        git_at(&full, &["checkout", "--quiet", &sha])?;
        let _ = writeln!(text, "{path}: {sha}");
        rows.push(json!({ "path": path, "commit": sha }));
    }
    Ok(Output::new(json!({ "submodules": rows }), text))
}

fn submodule_status(ws: &Workspace) -> Result<Output> {
    let mut text = String::new();
    let mut rows = Vec::new();
    for (path, sha) in pinned(ws)? {
        let full = ws.root.join(&path);
        let (mark, head) = if full.join(".git").exists() {
            let head = git_at(&full, &["rev-parse", "HEAD"]).unwrap_or_default();
            (if head == sha { " " } else { "+" }, head)
        } else {
            ("-", sha.clone())
        };
        let _ = writeln!(text, "{mark}{head} {path}");
        rows.push(json!({ "path": path, "pinned": sha, "current": head, "state": mark }));
    }
    Ok(Output::new(json!({ "submodules": rows }), text))
}

fn submodule_add(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    url: &str,
    path: &str,
) -> Result<Output> {
    let full = ws.root.join(path);
    wc::safe_join(&ws.root, path)?;
    let target = full.display().to_string();
    Command::new("git")
        .arg("-C")
        .arg(&ws.root)
        .args(["clone", "--quiet", "--", url, &target])
        .status()
        .context("running git")?;
    let sha = git_at(&full, &["rev-parse", "HEAD"])?;
    let modules = ws.root.join(".gitmodules");
    let mut text = std::fs::read_to_string(&modules).unwrap_or_default();
    let _ = writeln!(
        text,
        "[submodule \"{path}\"]\n\tpath = {path}\n\turl = {url}"
    );
    std::fs::write(&modules, text)?;
    let blob = ws.store.write(BLOB, sha.as_bytes())?;
    wc::add_gitlink(&ws.root, path, blob)?;
    ws.snapshot(actor, prov)?;
    Ok(Output::new(
        json!({ "path": path, "commit": sha }),
        format!("Added submodule {path} at {sha}\n"),
    ))
}

pub fn submodule(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    cmd: &SubmoduleCommand,
) -> Result<Output> {
    match cmd {
        SubmoduleCommand::Add { url, path } => submodule_add(ws, actor, prov, url, path),
        SubmoduleCommand::Update { init, .. } => submodule_update(ws, *init),
        SubmoduleCommand::Status => submodule_status(ws),
        SubmoduleCommand::Foreach { command } => {
            let line = command.join(" ");
            let mut text = String::new();
            for path in pinned(ws)?.keys() {
                let full = ws.root.join(path);
                let out = hooks::shell(&line, &full)
                    .output()
                    .with_context(|| format!("running {line} in {path}"))?;
                let _ = writeln!(text, "Entering '{path}'");
                text.push_str(&String::from_utf8_lossy(&out.stdout));
            }
            Ok(Output::new(json!({ "ran": line }), text))
        }
    }
}

// ------------------------------------------------------------- integrity

pub fn fsck(ws: &Workspace) -> Result<Output> {
    let report = ws.fsck()?;
    let mut text = format!(
        "Checked {} commit(s), {} tree(s), {} file version(s).\n",
        report.commits, report.trees, report.blobs
    );
    for problem in &report.problems {
        let _ = writeln!(text, "  ! {problem}");
    }
    if report.problems.is_empty() {
        text.push_str("No problems found.\n");
    }
    let failed = !report.problems.is_empty();
    Ok(Output::with_exit(json!(report), text, u8::from(failed)))
}

pub fn stats(ws: &Workspace) -> Result<Output> {
    let s = ws.store_stats()?;
    let text = format!(
        "loose objects: {}\nloose size: {} KiB\nin packs: {}\npacks: {}\npack size: {} KiB\n",
        s.loose_objects,
        s.loose_bytes / 1024,
        s.packed_objects,
        s.packs,
        s.pack_bytes / 1024
    );
    Ok(Output::new(json!(s), text))
}

pub fn verify(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    revision: &str,
) -> Result<Output> {
    let (payload, signature) = ws.look(actor, prov, |tx| {
        if let Some(tag) = tx.view.tags.get(revision).cloned() {
            let signature = tag
                .signature
                .clone()
                .ok_or_else(|| invalid(format!("tag `{revision}` is not signed")))?;
            let mut unsigned = tag;
            unsigned.signature = None;
            return Ok((serde_json::to_vec(&unsigned)?, signature));
        }
        let id = tx.resolve(revision)?;
        let mut commit = tx.commit(&id)?;
        let signature = commit
            .signature
            .take()
            .ok_or_else(|| invalid(format!("{} is not signed", id.short())))?;
        Ok((serde_json::to_vec(&commit)?, signature))
    })?;
    crate::sign::verify(ws, &payload, &signature)
}

// --------------------------------------------------------- external tools

fn tool(ws: &Workspace, key: &str) -> Result<String> {
    ws.config_get(key)?
        .or_else(|| crate::settings::get(key))
        .ok_or_else(|| anyhow!("set `{key}` to a command (`$LOCAL`, `$REMOTE`, `$MERGED`)"))
}

pub fn difftool(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Output> {
    let command = tool(ws, "diff.tool")?;
    let dir = tempfile::tempdir()?;
    let pairs = ws.read(actor, prov, |tx| {
        let from = tx.resolve(from.unwrap_or("@-"))?;
        let to = tx.resolve(to.unwrap_or("@"))?;
        let (a, b) = (tx.commit_tree(&from)?, tx.commit_tree(&to)?);
        let mut out = Vec::new();
        for (path, _) in hebra_core::diff::changed_paths(&a, &b) {
            let read = |tree: &hebra_core::model::Tree| -> Coded<Vec<u8>> {
                match tree.files.get(&path) {
                    Some(e) => tx.store.read(BLOB, &e.blob),
                    None => Ok(Vec::new()),
                }
            };
            out.push((path.clone(), read(&a)?, read(&b)?));
        }
        Ok(out)
    })?;
    for (path, old, new) in &pairs {
        let (l, r) = (dir.path().join("LOCAL"), dir.path().join("REMOTE"));
        std::fs::write(&l, old)?;
        std::fs::write(&r, new)?;
        let status = hooks::shell(&command, &ws.root)
            .env("LOCAL", &l)
            .env("REMOTE", &r)
            .env("MERGED", path)
            .status()
            .context("running the diff tool")?;
        if !status.success() {
            bail!("the diff tool failed on {path}");
        }
    }
    Ok(Output::new(
        json!({ "files": pairs.len() }),
        format!("Compared {} file(s).\n", pairs.len()),
    ))
}

pub fn mergetool(ws: &mut Workspace, actor: &Actor, prov: &Provenance) -> Result<Output> {
    let command = tool(ws, "merge.tool")?;
    let conflicts: Vec<String> = ws.read(actor, prov, |tx| {
        let wc = tx.wc_id()?;
        Ok(tx
            .commit(&wc)?
            .conflicts
            .into_iter()
            .map(|c| c.path)
            .collect())
    })?;
    for path in &conflicts {
        let status = hooks::shell(&command, &ws.root)
            .env("MERGED", ws.root.join(path))
            .status()
            .context("running the merge tool")?;
        if !status.success() {
            bail!("the merge tool failed on {path}");
        }
    }
    Ok(Output::new(
        json!({ "files": conflicts }),
        format!(
            "Opened {} file(s). Markers are detected automatically.\n",
            conflicts.len()
        ),
    ))
}

/// What a `config` command does in the global settings.
pub fn config_global(
    key: Option<&str>,
    value: Option<&str>,
    unset: bool,
    list: bool,
) -> Result<Output> {
    let settings = crate::settings::all();
    if list || key.is_none() {
        let text: String = settings
            .iter()
            .map(|(k, v)| format!("{k} = {v}\n"))
            .collect();
        return Ok(Output::new(json!(settings), text));
    }
    let key = key.unwrap_or_default();
    if unset {
        crate::settings::unset(key)?;
        return Ok(Output::new(
            json!({ "unset": key }),
            format!("Unset {key}\n"),
        ));
    }
    if let Some(value) = value {
        crate::settings::set(key, value)?;
        return Ok(Output::new(
            json!({ "key": key, "value": value }),
            format!("Set {key} = {value}\n"),
        ));
    }
    match settings.get(key) {
        Some(value) => Ok(Output::new(
            json!({ "key": key, "value": value }),
            format!("{value}\n"),
        )),
        None => bail!("config key `{key}` is not set"),
    }
}
