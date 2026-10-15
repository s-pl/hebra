//! Command implementations. Each returns an [`Output`] with both a JSON value
//! (for agents) and text (for people), so the CLI and the MCP server share
//! exactly the same behaviour.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use hebra_core::claims::{self, Claim};
use hebra_core::diff::{self, FileDiff, FileStatus};
use hebra_core::identity;
use hebra_core::model::{Actor, ActorKind, Commit, Conflict, Provenance};
use hebra_core::repo::DEFAULT_WORKSPACE;
use hebra_core::{Error, ObjectId, Tx, Workspace};
use serde_json::{Value, json};

use crate::cli::{
    BookmarkCommand, ChecksCommand, Command, ConfigCommand, DiffArgs, GitCommand, OpCommand,
    RemoteCommand, ReviewCommand, StartArgs, TaskCommand, WorkspaceCommand,
};
use crate::fmt::{ago, parse_ttl, slug};

pub struct Output {
    pub json: Value,
    pub text: String,
}

impl Output {
    pub(crate) fn new(json: Value, text: impl Into<String>) -> Self {
        Self {
            json,
            text: text.into(),
        }
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// `user.name` from Hebra's config, falling back to Git's.
fn configured_user(ws: Option<&Workspace>) -> Option<String> {
    if let Some(name) = ws.and_then(|w| w.config_get("user.name").ok().flatten()) {
        return Some(name);
    }
    let out = std::process::Command::new("git")
        .args(["config", "--get", "user.name"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub(crate) fn identity_for(ws: &Workspace) -> Result<(Actor, Provenance)> {
    let info = ws.info()?;
    let actor = identity::resolve_actor(env, info.as_ref(), configured_user(Some(ws)));
    let provenance = identity::provenance(env, info.as_ref());
    Ok((actor, provenance))
}

pub fn execute(command: Command, cwd: &Path) -> Result<Output> {
    if let Command::Init { path } = &command {
        return init(&cwd.join(path.as_deref().unwrap_or(Path::new("."))));
    }
    let mut ws = Workspace::open(cwd)?;
    let (actor, prov) = identity_for(&ws)?;
    let name = command_name(&command);
    match command {
        Command::Init { .. } | Command::Mcp => unreachable!("handled by the caller"),
        Command::Status => {
            ws.snapshot_others(&prov)?;
            let mut out = ws.run(&actor, &prov, name, status)?;
            add_remote_warnings(&mut ws, &actor, &prov, &mut out);
            Ok(out)
        }
        Command::Diff(args) => ws
            .run(&actor, &prov, name, |tx| diff_cmd(tx, &args))
            .map_err(Into::into),
        Command::Describe { revision, message } => ws
            .run(&actor, &prov, name, |tx| describe(tx, &revision, message))
            .map_err(Into::into),
        Command::Commit { message, task } => {
            let mut prov = prov;
            if let Some(task) = task {
                prov.task = Some(task);
            }
            ws.run(&actor, &prov, name, |tx| commit(tx, message))
                .map_err(Into::into)
        }
        Command::New { revisions, message } => ws
            .run(&actor, &prov, name, |tx| new(tx, &revisions, message))
            .map_err(Into::into),
        Command::Log { limit } => ws
            .run(&actor, &prov, name, |tx| log(tx, limit))
            .map_err(Into::into),
        Command::Show { revision } => ws
            .run(&actor, &prov, name, |tx| show(tx, &revision))
            .map_err(Into::into),
        Command::Merge { revisions, message } => ws
            .run(&actor, &prov, name, |tx| merge(tx, &revisions, message))
            .map_err(Into::into),
        Command::Resolve { paths } => ws
            .run(&actor, &prov, name, |tx| resolve(tx, &paths))
            .map_err(Into::into),
        Command::Start(args) => ws
            .run(&actor, &prov, name, |tx| start(tx, args))
            .map_err(Into::into),
        Command::Workspaces => {
            ws.snapshot_others(&prov)?;
            let activity: BTreeMap<String, String> = ws.last_activity()?.into_iter().collect();
            ws.run(&actor, &prov, name, |tx| workspaces(tx, &activity))
                .map_err(Into::into)
        }
        Command::Workspace(WorkspaceCommand::Forget { name: target }) => ws
            .run(&actor, &prov, name, |tx| forget(tx, &target))
            .map_err(Into::into),
        Command::Undo => ws.run(&actor, &prov, name, undo).map_err(Into::into),
        Command::Op(OpCommand::Log { limit }) => op_log(&ws, limit),
        Command::Op(OpCommand::Restore { id }) => ws
            .run(&actor, &prov, name, |tx| op_restore(tx, id))
            .map_err(Into::into),
        Command::Bookmark(cmd) => ws
            .run(&actor, &prov, name, |tx| bookmark(tx, cmd))
            .map_err(Into::into),
        Command::Claim { paths, ttl, note } => {
            let ttl = parse_ttl(&ttl)?;
            ws.run(&actor, &prov, name, |tx| claim(tx, &paths, ttl, note))
                .map_err(Into::into)
        }
        Command::Release { paths } => ws
            .run(&actor, &prov, name, |tx| release(tx, &paths))
            .map_err(Into::into),
        Command::Claims => ws.run(&actor, &prov, name, list_claims).map_err(Into::into),
        Command::Config(cmd) => config(&ws, cmd),
        Command::Remote(RemoteCommand::Add { name, url, token }) => {
            crate::remote::add(&ws, &name, &url, token)
        }
        Command::Remote(RemoteCommand::List) => crate::remote::list(&ws),
        Command::Remote(RemoteCommand::Remove { name }) => crate::remote::remove(&ws, &name),
        Command::Push {
            remote,
            delete: Some(name),
            ..
        } => crate::collab::delete_bookmark(&mut ws, &actor, &prov, &remote, &name),
        Command::Push {
            remote,
            bookmark,
            revision,
            delete: None,
        } => crate::remote::push(&mut ws, &actor, &prov, &remote, bookmark, revision),
        Command::Review(ReviewCommand::Open {
            bookmark,
            title,
            body,
            target,
            remote,
        }) => crate::collab::review_open(&ws, &remote, &bookmark, &target, &title, body),
        Command::Review(ReviewCommand::List { all, remote }) => {
            crate::collab::review_list(&ws, &remote, all)
        }
        Command::Review(ReviewCommand::Merge { id, remote }) => {
            crate::collab::review_act(&ws, &remote, id, "merge")
        }
        Command::Review(ReviewCommand::Close { id, remote }) => {
            crate::collab::review_act(&ws, &remote, id, "close")
        }
        Command::Watch { remote, interval } => {
            let json_out = std::env::var("HB_JSON").is_ok_and(|v| v == "1" || v == "true");
            crate::collab::watch(&mut ws, &actor, &prov, &remote, interval, json_out)
        }
        Command::Pull { remote } => crate::remote::pull(&mut ws, &actor, &prov, &remote),
        Command::Presence { remote } => crate::remote::presence(&mut ws, &actor, &prov, &remote),
        Command::Review(ReviewCommand::Show { id, remote }) => {
            crate::work::review_show(&ws, &remote, id)
        }
        Command::Review(ReviewCommand::Comment {
            id,
            body,
            path,
            line,
            remote,
        }) => crate::work::review_comment(&ws, &remote, id, &body, path, line),
        Command::Review(ReviewCommand::Approve {
            id,
            changes,
            note,
            remote,
        }) => crate::work::review_approve(&ws, &remote, id, changes, note),
        Command::Task(cmd) => task(&ws, cmd),
        Command::VerifyLog { remote, key } => {
            crate::verify_log::verify(&ws, &remote, key.as_deref())
        }
        Command::Checks(ChecksCommand::List { revision, remote }) => {
            crate::work::checks_list(&ws, &remote, revision.as_deref())
        }
        Command::Checks(ChecksCommand::Log { id, remote }) => {
            crate::work::checks_log(&ws, &remote, &id)
        }
        Command::Checks(ChecksCommand::Rerun {
            revision,
            name,
            remote,
        }) => crate::work::checks_act(&ws, &remote, "rerun", &revision, name),
        Command::Checks(ChecksCommand::Approve { revision, remote }) => {
            crate::work::checks_act(&ws, &remote, "approve", &revision, None)
        }
        Command::Login { .. }
        | Command::Logout { .. }
        | Command::Whoami { .. }
        | Command::Mandate(_)
        | Command::Runner(_)
        | Command::Org(_)
        | Command::Agent(_)
        | Command::Repo(_) => {
            unreachable!("server-wide commands are handled before opening a workspace")
        }
        Command::Git(GitCommand::Export {
            dir,
            branch,
            revision,
        }) => {
            let dir = cwd.join(dir);
            ws.run(&actor, &prov, name, |tx| {
                git_export(tx, &dir, &branch, &revision)
            })
            .map_err(Into::into)
        }
        Command::Git(GitCommand::Import {
            dir,
            branch,
            bookmark,
        }) => {
            let dir = cwd.join(dir);
            let bookmark = bookmark.unwrap_or_else(|| branch.clone());
            ws.run(&actor, &prov, name, |tx| {
                git_import(tx, &dir, &branch, &bookmark)
            })
            .map_err(Into::into)
        }
    }
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Init { .. } => "init",
        Command::Status => "status",
        Command::Diff(_) => "diff",
        Command::Describe { .. } => "describe",
        Command::Commit { .. } => "commit",
        Command::New { .. } => "new",
        Command::Log { .. } => "log",
        Command::Show { .. } => "show",
        Command::Merge { .. } => "merge",
        Command::Resolve { .. } => "resolve",
        Command::Start(_) => "start",
        Command::Workspaces => "workspaces",
        Command::Workspace(_) => "workspace forget",
        Command::Undo => "undo",
        Command::Op(_) => "op restore",
        Command::Bookmark(_) => "bookmark",
        Command::Claim { .. } => "claim",
        Command::Release { .. } => "release",
        Command::Claims => "claims",
        Command::Config(_) => "config",
        Command::Mcp => "mcp",
        Command::Remote(_) => "remote",
        Command::Push { .. } => "push",
        Command::Review(_) => "review",
        Command::Watch { .. } => "watch",
        Command::Pull { .. } => "pull",
        Command::Presence { .. } => "presence",
        Command::Task(_) => "task",
        Command::VerifyLog { .. } => "verify-log",
        Command::Checks(_) => "checks",
        Command::Login { .. } => "login",
        Command::Logout { .. } => "logout",
        Command::Whoami { .. } => "whoami",
        Command::Mandate(_) => "mandate",
        Command::Runner(_) => "runner",
        Command::Org(_) => "org",
        Command::Agent(_) => "agent",
        Command::Repo(_) => "repo",
        Command::Git(GitCommand::Export { .. }) => "git export",
        Command::Git(GitCommand::Import { .. }) => "git import",
    }
}

// ---------------------------------------------------------------- helpers

fn first_line(description: &str) -> &str {
    description.lines().next().unwrap_or("")
}

fn describe_or_placeholder(description: &str) -> String {
    if description.trim().is_empty() {
        "(no description)".to_owned()
    } else {
        first_line(description).to_owned()
    }
}

pub(crate) fn commit_json(id: &ObjectId, c: &Commit) -> Value {
    json!({
        "commit_id": id.hex(),
        "change_id": c.change_id.encode(),
        "description": c.description,
        "author": c.author,
        "timestamp": c.timestamp,
        "parents": c.parents.iter().map(ObjectId::hex).collect::<Vec<_>>(),
        "provenance": c.provenance,
        "conflicts": c.conflicts,
    })
}

pub(crate) fn short_line(id: &ObjectId, c: &Commit) -> String {
    format!(
        "{} {}  {}",
        c.change_id.short(),
        id.short(),
        describe_or_placeholder(&c.description)
    )
}

/// Files changed by a commit relative to its parents.
pub(crate) fn own_changes(
    tx: &mut Tx<'_>,
    id: &ObjectId,
) -> hebra_core::Result<Vec<(String, FileStatus)>> {
    let commit = tx.commit(id)?;
    let (base, _) = tx.parents_tree(&commit.parents)?;
    let tree = tx.tree(&commit.tree)?;
    Ok(diff::changed_paths(&base, &tree))
}

fn conflicts_text(out: &mut String, conflicts: &[Conflict]) {
    if conflicts.is_empty() {
        return;
    }
    let _ = writeln!(out, "Conflicts (recorded, not blocking):");
    for c in conflicts {
        let kind = serde_json::to_value(c.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let _ = writeln!(out, "  ! {} ({kind})", c.path);
    }
}

fn diff_text(out: &mut String, files: &[FileDiff], stat: bool) {
    if files.is_empty() {
        let _ = writeln!(out, "No changes.");
        return;
    }
    for f in files {
        if stat || f.binary {
            let detail = if f.binary {
                "binary".to_owned()
            } else {
                format!("+{} -{}", f.additions, f.deletions)
            };
            let _ = writeln!(out, "{} {}  {detail}", f.status.letter(), f.path);
        } else if let Some(patch) = &f.patch {
            out.push_str(patch);
        }
    }
}

// ---------------------------------------------------------------- commands

fn init(path: &Path) -> Result<Output> {
    std::fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
    let env_actor = identity::resolve_actor(env, None, configured_user(None));
    let prov = identity::provenance(env, None);
    let mut ws = Workspace::init(path, &env_actor, &prov)?;
    ws.snapshot(&env_actor, &prov)?;
    let root = ws.root.display().to_string();
    ws.run(&env_actor, &prov, "init", |tx| {
        let wc = tx.wc_id()?;
        let commit = tx.commit(&wc)?;
        let files = tx.tree(&commit.tree)?.files.len();
        let text = format!(
            "Initialized Hebra repository in {root}\nImported {files} file(s) into change {}.\n",
            commit.change_id.short()
        );
        Ok(Output::new(
            json!({ "root": root, "workspace": DEFAULT_WORKSPACE, "imported_files": files, "working_copy": commit_json(&wc, &commit) }),
            text,
        ))
    })
    .map_err(Into::into)
}

fn status(tx: &mut Tx<'_>) -> hebra_core::Result<Output> {
    let wc = tx.wc_id()?;
    let commit = tx.commit(&wc)?;
    let changes = own_changes(tx, &wc)?;
    let changed: BTreeSet<&String> = changes.iter().map(|(p, _)| p).collect();

    let claims = tx.claims()?;
    let claimed: Vec<(&String, &Claim)> = changed
        .iter()
        .flat_map(|path| {
            claims
                .iter()
                .filter(|c| c.workspace != tx.ws_name && c.covers(path))
                .map(move |c| (*path, c))
        })
        .collect();

    let others: Vec<(String, hebra_core::model::WorkspaceInfo)> = tx
        .view
        .workspaces
        .iter()
        .filter(|(name, _)| name.as_str() != tx.ws_name)
        .map(|(n, w)| (n.clone(), w.clone()))
        .collect();
    let mut overlapping = Vec::new();
    for (name, info) in &others {
        for (path, _) in own_changes(tx, &info.wc)? {
            if changed.contains(&path) {
                overlapping.push((path, name.clone(), info.actor.clone()));
            }
        }
    }

    let mut parents = Vec::new();
    for p in &commit.parents {
        let pc = tx.commit(p)?;
        parents.push((*p, pc));
    }

    let mut text = String::new();
    let _ = writeln!(text, "Workspace:    {}", tx.ws_name);
    let _ = writeln!(text, "Working copy: {}", short_line(&wc, &commit));
    for (id, pc) in &parents {
        let _ = writeln!(text, "Parent:       {}", short_line(id, pc));
    }
    if changes.is_empty() {
        let _ = writeln!(text, "No changes in the working copy.");
    } else {
        let _ = writeln!(text, "Changes:");
        for (path, st) in &changes {
            let _ = writeln!(text, "  {} {path}", st.letter());
        }
    }
    conflicts_text(&mut text, &commit.conflicts);
    if !claimed.is_empty() || !overlapping.is_empty() {
        let _ = writeln!(text, "Coordination warnings:");
        for (path, c) in &claimed {
            let _ = writeln!(
                text,
                "  {path} is claimed by {} in workspace {} ({})",
                c.actor.display(),
                c.workspace,
                c.note.as_deref().unwrap_or("no note")
            );
        }
        for (path, name, actor) in &overlapping {
            let who = actor
                .as_ref()
                .map_or_else(|| "someone".to_owned(), Actor::display);
            let _ = writeln!(
                text,
                "  {path} is also being changed in workspace {name} by {who}"
            );
        }
    }

    let json = json!({
        "workspace": tx.ws_name,
        "working_copy": commit_json(&wc, &commit),
        "parents": parents.iter().map(|(id, c)| commit_json(id, c)).collect::<Vec<_>>(),
        "changes": changes.iter().map(|(p, s)| json!({ "path": p, "status": s })).collect::<Vec<_>>(),
        "conflicts": commit.conflicts,
        "warnings": {
            "claimed_by_others": claimed.iter().map(|(p, c)| json!({ "path": p, "claim": c })).collect::<Vec<_>>(),
            "overlapping": overlapping.iter().map(|(p, n, a)| json!({ "path": p, "workspace": n, "actor": a })).collect::<Vec<_>>(),
        },
    });
    Ok(Output::new(json, text))
}

fn diff_cmd(tx: &mut Tx<'_>, args: &DiffArgs) -> hebra_core::Result<Output> {
    let (from_tree, to_tree) = if args.from.is_some() || args.to.is_some() {
        let from = tx.resolve(args.from.as_deref().unwrap_or("@-"))?;
        let to = tx.resolve(args.to.as_deref().unwrap_or("@"))?;
        (tx.commit_tree(&from)?, tx.commit_tree(&to)?)
    } else {
        let id = tx.resolve(args.revision.as_deref().unwrap_or("@"))?;
        let commit = tx.commit(&id)?;
        (tx.parents_tree(&commit.parents)?.0, tx.tree(&commit.tree)?)
    };
    let files = diff::diff_trees(tx.store, &from_tree, &to_tree, !args.stat)?;
    let mut text = String::new();
    diff_text(&mut text, &files, args.stat);
    Ok(Output::new(json!({ "files": files }), text))
}

fn describe(tx: &mut Tx<'_>, revision: &str, message: String) -> hebra_core::Result<Output> {
    let id = tx.resolve(revision)?;
    if id == tx.root_commit {
        return Err(Error::InvalidArgument(
            "the root commit cannot be modified".to_owned(),
        ));
    }
    // Describing an empty change makes you its author.
    let adopt = tx.is_empty_change(&id)?;
    let actor = tx.actor.clone();
    let new = tx.rewrite(id, |c| {
        if adopt {
            c.author = actor;
        }
        c.description = message;
    })?;
    let commit = tx.commit(&new)?;
    let text = format!("Described {}\n", short_line(&new, &commit));
    Ok(Output::new(commit_json(&new, &commit), text))
}

fn commit(tx: &mut Tx<'_>, message: String) -> hebra_core::Result<Output> {
    let wc = tx.wc_id()?;
    let provenance = tx.provenance.clone();
    let described = tx.rewrite(wc, |c| {
        c.description = message;
        if c.provenance.is_empty() {
            c.provenance = provenance;
        }
    })?;
    let tree = tx.commit(&described)?.tree;
    let next = tx.new_commit(vec![described], tree, String::new(), Vec::new())?;
    tx.switch_wc(next)?;
    let done = tx.commit(&described)?;
    let next_commit = tx.commit(&next)?;
    let text = format!(
        "Committed {}\nWorking copy is now {}\n",
        short_line(&described, &done),
        short_line(&next, &next_commit)
    );
    Ok(Output::new(
        json!({ "committed": commit_json(&described, &done), "working_copy": commit_json(&next, &next_commit) }),
        text,
    ))
}

fn new_change(
    tx: &mut Tx<'_>,
    parents: Vec<ObjectId>,
    message: Option<String>,
) -> hebra_core::Result<(ObjectId, Commit)> {
    let parents = tx.simplify_parents(parents)?;
    let (tree, conflicts) = tx.parents_tree(&parents)?;
    let tree_id = tx.store.write_json(hebra_core::store::TREE, &tree)?;
    let id = tx.new_commit(parents, tree_id, message.unwrap_or_default(), conflicts)?;
    tx.switch_wc(id)?;
    let commit = tx.commit(&id)?;
    Ok((id, commit))
}

fn new(
    tx: &mut Tx<'_>,
    revisions: &[String],
    message: Option<String>,
) -> hebra_core::Result<Output> {
    let mut parents = Vec::new();
    for rev in revisions {
        let id = tx.resolve(rev)?;
        if !parents.contains(&id) {
            parents.push(id);
        }
    }
    let (id, commit) = new_change(tx, parents, message)?;
    let mut text = format!("Working copy is now {}\n", short_line(&id, &commit));
    conflicts_text(&mut text, &commit.conflicts);
    Ok(Output::new(commit_json(&id, &commit), text))
}

fn merge(
    tx: &mut Tx<'_>,
    revisions: &[String],
    message: Option<String>,
) -> hebra_core::Result<Output> {
    let wc = tx.wc_id()?;
    // An empty working copy is replaced by the merge instead of kept as a parent.
    let mut parents = if tx.is_empty_change(&wc)? {
        tx.commit(&wc)?.parents
    } else {
        vec![wc]
    };
    let original = parents.clone();
    let mut names = Vec::new();
    for rev in revisions {
        let id = tx.resolve(rev)?;
        if !parents.contains(&id) {
            parents.push(id);
        }
        let c = tx.commit(&id)?;
        names.push(format!(
            "{} ({})",
            c.change_id.short(),
            describe_or_placeholder(&c.description)
        ));
    }
    // A single remaining parent is a fast-forward: the new change simply
    // starts on top of the merged work.
    let parents = tx.simplify_parents(parents)?;
    if parents == original {
        return Err(Error::InvalidArgument(
            "nothing to merge: the revisions are already included".to_owned(),
        ));
    }
    let message = message.unwrap_or_else(|| format!("merge {}", names.join(", ")));
    let (id, commit) = new_change(tx, parents, Some(message))?;
    let mut text = format!("Merged into {}\n", short_line(&id, &commit));
    if commit.conflicts.is_empty() {
        let _ = writeln!(text, "No conflicts.");
    } else {
        conflicts_text(&mut text, &commit.conflicts);
        let _ = writeln!(
            text,
            "Edit the files to resolve; markers are detected automatically."
        );
    }
    Ok(Output::new(commit_json(&id, &commit), text))
}

fn resolve(tx: &mut Tx<'_>, paths: &[String]) -> hebra_core::Result<Output> {
    let wc = tx.wc_id()?;
    let before = tx.commit(&wc)?.conflicts;
    let remaining: Vec<Conflict> = if paths.is_empty() {
        Vec::new()
    } else {
        before
            .iter()
            .filter(|c| !paths.contains(&c.path))
            .cloned()
            .collect()
    };
    let resolved = before.len() - remaining.len();
    let new = tx.rewrite(wc, |c| c.conflicts = remaining)?;
    let commit = tx.commit(&new)?;
    let mut text = format!("Marked {resolved} conflict(s) as resolved.\n");
    conflicts_text(&mut text, &commit.conflicts);
    Ok(Output::new(
        json!({ "resolved": resolved, "remaining": commit.conflicts }),
        text,
    ))
}

fn start(tx: &mut Tx<'_>, args: StartArgs) -> hebra_core::Result<Output> {
    let base = tx.resolve(&args.from)?;
    let wanted = args.name.unwrap_or_else(|| slug(&args.task));
    let mut name = wanted.clone();
    let mut n = 2;
    while tx.view.workspaces.contains_key(&name) {
        name = format!("{wanted}-{n}");
        n += 1;
    }
    let actor = match (args.agent, &tx.actor.kind) {
        (Some(agent), _) => Some(Actor {
            name: agent,
            kind: ActorKind::Agent,
            on_behalf_of: args.on_behalf_of.or_else(|| match tx.actor.kind {
                ActorKind::Human => Some(tx.actor.name.clone()),
                ActorKind::Agent => tx.actor.on_behalf_of.clone(),
            }),
        }),
        (None, ActorKind::Agent) => Some(tx.actor.clone()),
        (None, ActorKind::Human) => None,
    };
    let default_dir = tx
        .repo_dir
        .parent()
        .unwrap_or(tx.repo_dir)
        .join("workspaces")
        .join(&name);
    let path = args.path.unwrap_or(default_dir);
    let info = tx.create_workspace(&name, &path, base, actor, Some(args.task.clone()))?;
    let commit = tx.commit(&info.wc)?;
    let text = format!(
        "Started workspace {name} for: {}\n  path:   {}\n  change: {}\n  base:   {}\nWork in that directory; every change is recorded automatically.\nWhen done: `hb commit -m \"...\"` there, then `hb merge {name}@` from your workspace.\n",
        args.task,
        info.path,
        commit.change_id.short(),
        base.short()
    );
    Ok(Output::new(
        json!({ "workspace": name, "path": info.path, "actor": info.actor, "task": info.task, "working_copy": commit_json(&info.wc, &commit), "base": base.hex() }),
        text,
    ))
}

fn workspaces(tx: &mut Tx<'_>, activity: &BTreeMap<String, String>) -> hebra_core::Result<Output> {
    let entries: Vec<(String, hebra_core::model::WorkspaceInfo)> = tx
        .view
        .workspaces
        .iter()
        .map(|(n, w)| (n.clone(), w.clone()))
        .collect();
    let claims = tx.claims()?;
    let mut rows = Vec::new();
    let mut text = String::new();
    for (name, info) in entries {
        let commit = tx.commit(&info.wc)?;
        let changes = own_changes(tx, &info.wc)?;
        let ws_claims: Vec<&Claim> = claims.iter().filter(|c| c.workspace == name).collect();
        let last = activity.get(&name).cloned();
        let who = info
            .actor
            .as_ref()
            .map_or_else(|| "(human)".to_owned(), Actor::display);
        let marker = if name == tx.ws_name { "*" } else { " " };
        let _ = writeln!(
            text,
            "{marker} {name}  {who}  {}  {} file(s){}{}",
            describe_or_placeholder(&commit.description),
            changes.len(),
            if commit.conflicts.is_empty() {
                String::new()
            } else {
                format!(", {} conflict(s)", commit.conflicts.len())
            },
            last.as_deref()
                .map(|t| format!("  active {}", ago(t)))
                .unwrap_or_default(),
        );
        for (path, st) in &changes {
            let _ = writeln!(text, "      {} {path}", st.letter());
        }
        for c in &ws_claims {
            let _ = writeln!(text, "      claims {}", c.path);
        }
        rows.push(json!({
            "name": name,
            "current": name == tx.ws_name,
            "path": info.path,
            "actor": info.actor,
            "task": info.task,
            "working_copy": commit_json(&info.wc, &commit),
            "changes": changes.iter().map(|(p, s)| json!({ "path": p, "status": s })).collect::<Vec<_>>(),
            "claims": ws_claims,
            "last_activity": last,
        }));
    }
    Ok(Output::new(json!({ "workspaces": rows }), text))
}

fn forget(tx: &mut Tx<'_>, name: &str) -> hebra_core::Result<Output> {
    if name == tx.ws_name {
        return Err(Error::InvalidArgument(
            "cannot forget the current workspace".to_owned(),
        ));
    }
    let info = tx
        .view
        .workspaces
        .remove(name)
        .ok_or_else(|| Error::WorkspaceNotFound(name.to_owned()))?;
    // Keep its work reachable.
    if !tx.is_empty_change(&info.wc)? {
        tx.view.heads.insert(info.wc);
    }
    Ok(Output::new(
        json!({ "forgotten": name, "path": info.path }),
        format!(
            "Forgot workspace {name}. Its directory {} was left on disk.\n",
            info.path
        ),
    ))
}

fn log(tx: &mut Tx<'_>, limit: usize) -> hebra_core::Result<Output> {
    let current = tx.wc_id()?;
    let mut at: BTreeMap<ObjectId, Vec<String>> = BTreeMap::new();
    for (name, info) in &tx.view.workspaces {
        at.entry(info.wc).or_default().push(format!("{name}@"));
    }
    for (name, id) in &tx.view.bookmarks {
        at.entry(*id).or_default().push(name.clone());
    }
    let visible = tx.visible()?;
    let total = visible.len();
    let mut text = String::new();
    let mut entries = Vec::new();
    for id in visible.into_iter().take(limit) {
        let c = tx.commit(&id)?;
        let labels = at.get(&id).cloned().unwrap_or_default();
        let symbol = if id == current {
            "@"
        } else if id == tx.root_commit {
            "◆"
        } else if !c.conflicts.is_empty() {
            "×"
        } else {
            "○"
        };
        let when = if id == tx.root_commit {
            String::new()
        } else {
            ago(&c.timestamp)
        };
        let merge_note = if c.parents.len() > 1 { "  (merge)" } else { "" };
        let _ = writeln!(
            text,
            "{symbol}  {} {}  {}  {when}{}{merge_note}",
            c.change_id.short(),
            id.short(),
            if id == tx.root_commit {
                "root()".to_owned()
            } else {
                c.author.display()
            },
            if labels.is_empty() {
                String::new()
            } else {
                format!("  [{}]", labels.join(" "))
            },
        );
        let empty = id != tx.root_commit && tx.is_empty_change(&id)?;
        if id != tx.root_commit {
            let summary = if empty {
                "(empty)".to_owned()
            } else {
                describe_or_placeholder(&c.description)
            };
            let _ = writeln!(text, "│  {summary}");
            if let Some(task) = &c.provenance.task
                && task != first_line(&c.description)
            {
                let _ = writeln!(text, "│  task: {task}");
            }
        }
        let mut entry = commit_json(&id, &c);
        entry["labels"] = json!(labels);
        entry["current"] = json!(id == current);
        entry["empty"] = json!(empty);
        entries.push(entry);
    }
    if total > limit {
        let _ = writeln!(text, "… {} more (use -n)", total - limit);
    }
    Ok(Output::new(
        json!({ "commits": entries, "total": total }),
        text,
    ))
}

fn show(tx: &mut Tx<'_>, revision: &str) -> hebra_core::Result<Output> {
    let id = tx.resolve(revision)?;
    let c = tx.commit(&id)?;
    let (base, _) = tx.parents_tree(&c.parents)?;
    let tree = tx.tree(&c.tree)?;
    let files = diff::diff_trees(tx.store, &base, &tree, true)?;
    let mut text = String::new();
    let _ = writeln!(text, "Change:  {}", c.change_id);
    let _ = writeln!(text, "Commit:  {id}");
    let _ = writeln!(text, "Author:  {}", c.author.display());
    let _ = writeln!(text, "Date:    {} ({})", c.timestamp, ago(&c.timestamp));
    for p in &c.parents {
        let _ = writeln!(text, "Parent:  {}", p.short());
    }
    if let Some(task) = &c.provenance.task {
        let _ = writeln!(text, "Task:    {task}");
    }
    if let Some(session) = &c.provenance.session {
        let _ = writeln!(text, "Session: {session}");
    }
    if let Some(prompt) = &c.provenance.prompt {
        let _ = writeln!(text, "Prompt:  {prompt}");
    }
    let _ = writeln!(
        text,
        "\n    {}\n",
        describe_or_placeholder(&c.description).replace('\n', "\n    ")
    );
    conflicts_text(&mut text, &c.conflicts);
    diff_text(&mut text, &files, false);
    let mut json = commit_json(&id, &c);
    json["files"] = json!(files);
    Ok(Output::new(json, text))
}

fn undo(tx: &mut Tx<'_>) -> hebra_core::Result<Output> {
    let result = tx.undo()?;
    let text = format!("Undid operation {} ({}).\n", result.undone, result.command);
    Ok(Output::new(json!(result), text))
}

fn op_log(ws: &Workspace, limit: usize) -> Result<Output> {
    let ops = ws.operations(limit)?;
    let mut text = String::new();
    for op in &ops {
        let undoes = op
            .undoes
            .map(|u| format!(" (undoes {u})"))
            .unwrap_or_default();
        let _ = writeln!(
            text,
            "{:>5}  {:<9} {:<16} {}  {}  {}{undoes}",
            op.id,
            serde_json::to_value(op.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            op.command,
            op.workspace,
            op.actor.display(),
            ago(&op.time),
        );
    }
    Ok(Output::new(json!({ "operations": ops }), text))
}

fn op_restore(tx: &mut Tx<'_>, id: i64) -> hebra_core::Result<Output> {
    let checkout = tx.restore(id)?;
    Ok(Output::new(
        json!({ "restored": id, "checkout": checkout }),
        format!("Restored the repository to operation {id}.\n"),
    ))
}

fn bookmark(tx: &mut Tx<'_>, cmd: BookmarkCommand) -> hebra_core::Result<Output> {
    match cmd {
        BookmarkCommand::Set { name, revision } => {
            let id = tx.resolve(&revision)?;
            tx.view.bookmarks.insert(name.clone(), id);
            let c = tx.commit(&id)?;
            Ok(Output::new(
                json!({ "bookmark": name, "commit": commit_json(&id, &c) }),
                format!("Bookmark {name} -> {}\n", short_line(&id, &c)),
            ))
        }
        BookmarkCommand::Delete { name } => {
            tx.view
                .bookmarks
                .remove(&name)
                .ok_or_else(|| Error::RevisionNotFound(name.clone()))?;
            Ok(Output::new(
                json!({ "deleted": name }),
                format!("Deleted bookmark {name}\n"),
            ))
        }
        BookmarkCommand::List => {
            let marks: Vec<(String, ObjectId)> = tx
                .view
                .bookmarks
                .iter()
                .map(|(n, i)| (n.clone(), *i))
                .collect();
            let mut text = String::new();
            let mut rows = Vec::new();
            for (name, id) in marks {
                let c = tx.commit(&id)?;
                let _ = writeln!(text, "{name}: {}", short_line(&id, &c));
                rows.push(json!({ "name": name, "commit": commit_json(&id, &c) }));
            }
            Ok(Output::new(json!({ "bookmarks": rows }), text))
        }
    }
}

fn claim(
    tx: &mut Tx<'_>,
    paths: &[String],
    ttl: i64,
    note: Option<String>,
) -> hebra_core::Result<Output> {
    let now = claims::now();
    let existing = tx.claims()?;
    let mut text = String::new();
    let mut rows = Vec::new();
    let mut contested = Vec::new();
    for path in paths {
        let path = path.trim_start_matches("./").replace('\\', "/");
        for other in existing.iter().filter(|c| c.workspace != tx.ws_name) {
            if other.covers(&path) || other.path.starts_with(&format!("{path}/")) {
                contested.push(json!({ "path": path, "held_by": other }));
                let _ = writeln!(
                    text,
                    "  note: {path} overlaps a claim by {} ({})",
                    other.actor.display(),
                    other.workspace
                );
            }
        }
        let c = Claim {
            path: path.clone(),
            workspace: tx.ws_name.to_owned(),
            actor: tx.actor.clone(),
            note: note.clone(),
            created_at: now,
            expires_at: now + ttl,
        };
        tx.claim(&c)?;
        let _ = writeln!(text, "Claimed {path} for {}", crate::fmt::duration(ttl));
        rows.push(c);
    }
    Ok(Output::new(
        json!({ "claims": rows, "contested": contested }),
        text,
    ))
}

fn release(tx: &mut Tx<'_>, paths: &[String]) -> hebra_core::Result<Output> {
    let n = tx.release(paths)?;
    Ok(Output::new(
        json!({ "released": n }),
        format!("Released {n} claim(s).\n"),
    ))
}

fn list_claims(tx: &mut Tx<'_>) -> hebra_core::Result<Output> {
    let all = tx.claims()?;
    let now = claims::now();
    let mut text = String::new();
    if all.is_empty() {
        let _ = writeln!(text, "No active claims.");
    }
    for c in &all {
        let _ = writeln!(
            text,
            "{}  {}  {}  expires in {}{}",
            c.path,
            c.workspace,
            c.actor.display(),
            crate::fmt::duration(c.expires_at - now),
            c.note
                .as_deref()
                .map(|n| format!("  — {n}"))
                .unwrap_or_default()
        );
    }
    Ok(Output::new(json!({ "claims": all }), text))
}

fn config(ws: &Workspace, cmd: ConfigCommand) -> Result<Output> {
    match cmd {
        ConfigCommand::Get { key } => {
            let Some(value) = ws.config_get(&key)? else {
                bail!("config key `{key}` is not set");
            };
            Ok(Output::new(
                json!({ "key": key, "value": value }),
                format!("{value}\n"),
            ))
        }
        ConfigCommand::Set { key, value } => {
            ws.config_set(&key, &value)?;
            Ok(Output::new(
                json!({ "key": key, "value": value }),
                format!("Set {key} = {value}\n"),
            ))
        }
        ConfigCommand::List => {
            let all = ws.config_list()?;
            let text: String = all.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
            Ok(Output::new(json!(all), text))
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> hebra_core::Result<std::process::Output> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|source| Error::Io {
            path: "git".into(),
            source,
        })
}

fn git_error(what: &str, out: &std::process::Output) -> Error {
    Error::InvalidArgument(format!(
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

fn git_export(
    tx: &mut Tx<'_>,
    dir: &Path,
    branch: &str,
    revision: &str,
) -> hebra_core::Result<Output> {
    use std::io::Write as _;
    use std::process::{Command as Process, Stdio};

    let io = |source: std::io::Error| Error::Io {
        path: "git fast-import".into(),
        source,
    };
    let tip = tx.resolve(revision)?;
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let out = git(dir, &["init", "--bare", "--quiet"])?;
        if !out.status.success() {
            return Err(git_error("git init", &out));
        }
    } else if !git(dir, &["rev-parse", "--git-dir"])?.status.success() {
        return Err(Error::InvalidArgument(format!(
            "{} is not a Git repository",
            dir.display()
        )));
    }

    let mut child = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(["fast-import", "--quiet", "--force"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io)?;
    let stats = {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::InvalidArgument("git fast-import has no stdin".to_owned()))?;
        let mut buffered = std::io::BufWriter::new(stdin);
        let stats = hebra_core::gitexport::write_fast_import(tx, tip, branch, &mut buffered)?;
        buffered.flush().map_err(io)?;
        stats
        // stdin is dropped here, so fast-import sees end of input.
    };
    let out = child.wait_with_output().map_err(io)?;
    if !out.status.success() {
        return Err(git_error("git fast-import", &out));
    }
    let head = git(dir, &["rev-parse", &format!("refs/heads/{branch}")])?;
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    let text = format!(
        "Exported {} commit(s) and {} file version(s) to {} ({branch} -> {})\n",
        stats.commits,
        stats.blobs,
        dir.display(),
        sha.get(..12).unwrap_or(&sha)
    );
    Ok(Output::new(
        json!({
            "dir": dir.display().to_string(),
            "branch": branch,
            "git_commit": sha,
            "commits": stats.commits,
            "blobs": stats.blobs,
        }),
        text,
    ))
}

fn git_import(
    tx: &mut Tx<'_>,
    dir: &Path,
    branch: &str,
    bookmark: &str,
) -> hebra_core::Result<Output> {
    use std::process::{Command as Process, Stdio};

    let io = |source: std::io::Error| Error::Io {
        path: "git fast-export".into(),
        source,
    };
    let reference = format!("refs/heads/{branch}");
    if !git(dir, &["rev-parse", "--verify", "--quiet", &reference])?
        .status
        .success()
    {
        return Err(Error::InvalidArgument(format!(
            "{} has no branch `{branch}`",
            dir.display()
        )));
    }
    let mut child = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "fast-export",
            "--show-original-ids",
            "--signed-tags=strip",
            "--reencode=yes",
            &reference,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::InvalidArgument("git fast-export has no stdout".to_owned()))?;
    let (stats, tip) = hebra_core::gitimport::import(tx, std::io::BufReader::new(stdout))?;
    let out = child.wait_with_output().map_err(io)?;
    if !out.status.success() {
        return Err(git_error("git fast-export", &out));
    }
    let tip = tip.ok_or_else(|| Error::InvalidArgument(format!("branch `{branch}` is empty")))?;
    tx.view.bookmarks.insert(bookmark.to_owned(), tip);
    let commit = tx.commit(&tip)?;
    let mut text = format!(
        "Imported {} commit(s) and {} file version(s) from {} into bookmark {bookmark}\n  {}\n",
        stats.commits,
        stats.blobs,
        dir.display(),
        short_line(&tip, &commit)
    );
    if stats.skipped_entries > 0 {
        let _ = writeln!(
            text,
            "  note: {} symlink or submodule entries were skipped",
            stats.skipped_entries
        );
    }
    let _ = writeln!(text, "Start working on it with `hb new {bookmark}`.");
    Ok(Output::new(
        json!({ "bookmark": bookmark, "stats": stats, "tip": commit_json(&tip, &commit) }),
        text,
    ))
}

/// If an `origin` remote is configured, shares our presence and adds warnings
/// for files other machines are touching. Best effort: never fails `status`.
fn add_remote_warnings(ws: &mut Workspace, actor: &Actor, prov: &Provenance, out: &mut Output) {
    if ws.config_get("remote.origin.url").ok().flatten().is_none() {
        return;
    }
    let changed: Vec<String> = out.json["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["path"].as_str().map(str::to_owned))
        .collect();
    match crate::remote::exchange(ws, actor, prov, "origin", std::time::Duration::from_secs(3)) {
        Ok(others) => {
            let warnings = crate::remote::remote_warnings(&others, &changed);
            if !warnings.is_empty() {
                let _ = writeln!(out.text, "Other machines:");
                for w in &warnings {
                    let verb = if w["claimed"].as_bool().unwrap_or(false) {
                        "is claimed"
                    } else {
                        "is also being changed"
                    };
                    let _ = writeln!(
                        out.text,
                        "  {} {verb} by {} on {} (workspace {})",
                        w["path"].as_str().unwrap_or("?"),
                        w["actor"].as_str().unwrap_or("?"),
                        w["machine"].as_str().unwrap_or("?"),
                        w["workspace"].as_str().unwrap_or("?"),
                    );
                }
            }
            out.json["warnings"]["remote"] = json!(warnings);
        }
        Err(e) => {
            out.json["warnings"]["remote_error"] = json!(e.to_string());
        }
    }
}

fn task(ws: &Workspace, cmd: TaskCommand) -> Result<Output> {
    use crate::work::{NewTask, task_act, task_create, task_list, task_show};
    match cmd {
        TaskCommand::List { state, remote } => task_list(ws, &remote, &state),
        TaskCommand::Create {
            title,
            body,
            acceptance,
            labels,
            parent,
            after,
            remote,
        } => task_create(
            ws,
            &remote,
            NewTask {
                title,
                body,
                acceptance,
                labels,
                parent,
                after,
            },
        ),
        TaskCommand::Show { id, remote } => task_show(ws, &remote, id),
        TaskCommand::Claim {
            id,
            minutes,
            remote,
        } => task_act(
            ws,
            &remote,
            id,
            "claim",
            json!({ "for_secs": minutes * 60 }),
        ),
        TaskCommand::Release { id, remote } => task_act(ws, &remote, id, "release", json!({})),
        TaskCommand::Done { id, remote } => {
            task_act(ws, &remote, id, "close", json!({ "state": "done" }))
        }
        TaskCommand::Cancel { id, remote } => {
            task_act(ws, &remote, id, "close", json!({ "state": "cancelled" }))
        }
        TaskCommand::Reopen { id, remote } => task_act(ws, &remote, id, "reopen", json!({})),
    }
}
