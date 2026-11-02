//! The commands people know from Git, on Hebra's model: everything is
//! already recorded in the working-copy change, nothing stops half-way, and
//! every command can be undone.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::history::{PathSpec, Step, take_paths};
use hebra_core::merge::{self, Side};
use hebra_core::model::{Actor, Commit, Conflict, ConflictKind, Provenance, Tag, Tree};
use hebra_core::repo::now_timestamp;
use hebra_core::wc::DOT_DIR;
use hebra_core::{Error, ObjectId, Tx, Workspace};
use serde_json::{Value, json};

use crate::cli::{BranchArgs, CommitArgs, MergeArgs, RebaseArgs, StashCommand, TagArgs};
use crate::commands::{Output, commit_json, short_line};
use crate::fmt::parse_date;
use crate::{hooks, sign};

pub(crate) type Coded<T> = hebra_core::Result<T>;

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidArgument(message.into())
}

/// A path as the user wrote it, as a path from the workspace root.
pub(crate) fn tree_path(ws: &Workspace, cwd: &Path, path: &str) -> Result<String> {
    if path.contains('*') || path.contains('?') {
        return Ok(path.replace('\\', "/"));
    }
    let full = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let clean = normalize(&full);
    let root = normalize(&ws.root);
    let rel = clean
        .strip_prefix(&root)
        .map_err(|_| anyhow!("{path} is outside the workspace"))?;
    let text = rel.to_string_lossy().replace('\\', "/");
    Ok(if text.is_empty() {
        ".".to_owned()
    } else {
        text
    })
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn tree_paths(ws: &Workspace, cwd: &Path, paths: &[String]) -> Result<Vec<String>> {
    paths.iter().map(|p| tree_path(ws, cwd, p)).collect()
}

/// `Name <email>`.
pub(crate) fn parse_person(text: &str) -> Actor {
    match text.split_once('<') {
        Some((name, rest)) => Actor {
            email: rest.split('>').next().map(|e| e.trim().to_owned()),
            ..Actor::human(name.trim())
        },
        None => Actor::human(text.trim()),
    }
}

fn subject(description: &str) -> String {
    description.lines().next().unwrap_or("").to_owned()
}

// ---------------------------------------------------------------- commit

fn editor() -> Option<String> {
    ["HB_EDITOR", "GIT_EDITOR", "VISUAL", "EDITOR"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Lets the user edit `text`, for commands asked to without giving it on the
/// command line. Agents never get here: without an editor this fails.
pub(crate) fn edit_text(ws: &Workspace, name: &str, text: &str) -> Result<String> {
    use std::io::IsTerminal;
    let Some(editor) = editor() else {
        bail!("no text given and no editor configured (use -m, or set EDITOR)");
    };
    if !std::io::stdin().is_terminal() && std::env::var_os("HB_EDITOR").is_none() {
        bail!("no text given and this is not a terminal (use -m)");
    }
    let path = ws.root.join(DOT_DIR).join(name);
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    let status = hooks::shell(&format!("{editor} \"{}\"", path.display()), &ws.root)
        .status()
        .context("starting the editor")?;
    if !status.success() {
        bail!("the editor failed");
    }
    let edited = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(edited
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned())
}

fn commit_message(ws: &Workspace, args: &CommitArgs) -> Result<Option<String>> {
    if let Some(file) = &args.file {
        return Ok(Some(
            std::fs::read_to_string(file)
                .with_context(|| format!("reading {}", file.display()))?
                .trim_end()
                .to_owned(),
        ));
    }
    if !args.message.is_empty() {
        return Ok(Some(args.message.join("\n\n")));
    }
    if args.amend || args.fixup.is_some() || args.squash.is_some() {
        return Ok(None);
    }
    let text = edit_text(
        ws,
        "COMMIT_EDITMSG",
        "\n# Describe this change. Lines starting with # are ignored.\n",
    )?;
    if text.is_empty() {
        bail!("aborting: empty description");
    }
    Ok(Some(text))
}

fn advance_bookmark(tx: &mut Tx<'_>, done: ObjectId) -> Coded<()> {
    if let Some(name) = tx.current_bookmark()
        && let Some(target) = tx.view.bookmarks.get(&name).copied()
        && tx.is_ancestor(target, done)?
    {
        tx.view.bookmarks.insert(name, done);
    }
    Ok(())
}

pub fn commit(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    args: &CommitArgs,
    cwd: &Path,
) -> Result<Output> {
    if !args.no_verify {
        hooks::run_required(ws, "pre-commit", &[])?;
    }
    let mut message = commit_message(ws, args)?;
    if let Some(text) = &mut message
        && !args.no_verify
    {
        *text = hooks::commit_msg(ws, text)?;
    }
    let spec = PathSpec::new(&tree_paths(ws, cwd, &args.paths)?)?;
    let chosen = if args.patch {
        Some(choose_hunks(ws, actor, prov, &spec)?)
    } else {
        None
    };
    let mut prov = prov.clone();
    if let Some(task) = &args.task {
        prov.task = Some(task.clone());
    }
    let author = args.author.as_deref().map(parse_person);
    let date = args.date.as_deref().map(parse_date).transpose()?;
    let signer = sign::wanted(ws, &args.sign);

    let out = ws.run(actor, &prov, "commit", |tx| {
        let fixup = match (&args.fixup, &args.squash) {
            (Some(rev), _) => {
                let target = tx.resolve(rev)?;
                Some(format!("fixup! {}", subject(&tx.commit(&target)?.description)))
            }
            (_, Some(rev)) => {
                let target = tx.resolve(rev)?;
                let text = subject(&tx.commit(&target)?.description);
                Some(match &message {
                    Some(m) => format!("squash! {text}\n\n{m}"),
                    None => format!("squash! {text}"),
                })
            }
            _ => None,
        };
        let message = fixup.or_else(|| message.clone());
        let wc = tx.wc_id()?;
        let (done, next) = if args.amend {
            let parent = tx
                .commit(&wc)?
                .parents
                .first()
                .copied()
                .filter(|p| *p != tx.root_commit)
                .ok_or_else(|| invalid("there is no previous change to amend"))?;
            let amended = tx.squash(wc, parent, &spec, message)?;
            (amended, tx.wc_id()?)
        } else if let Some(chosen) = &chosen {
            let first = commit_hunks(tx, wc, chosen, message.unwrap_or_default())?;
            (first, tx.wc_id()?)
        } else if spec.is_all() {
            let text = message.unwrap_or_default();
            let provenance = tx.provenance.clone();
            let described = tx.rewrite(wc, |c| {
                c.description = text;
                if c.provenance.is_empty() {
                    c.provenance = provenance;
                }
            })?;
            let tree = tx.commit(&described)?.tree;
            let next = tx.new_commit(vec![described], tree, String::new(), Vec::new())?;
            tx.switch_wc(next)?;
            (described, next)
        } else {
            let first = tx.split(wc, &spec, message.unwrap_or_default())?;
            (first, tx.wc_id()?)
        };
        let done = stamp(tx, done, author.clone(), date.clone())?;
        let done = sign_one(tx, done, signer.as_ref())?;
        advance_bookmark(tx, done)?;
        let _ = next;
        let next = tx.wc_id()?;
        let committed = tx.commit(&done)?;
        let working = tx.commit(&next)?;
        let text = format!(
            "Committed {}\nWorking copy is now {}\n",
            short_line(&done, &committed),
            short_line(&next, &working)
        );
        Ok(Output::new(
            json!({ "committed": commit_json(&done, &committed), "working_copy": commit_json(&next, &working) }),
            text,
        ))
    })?;
    if !args.no_verify {
        hooks::run_optional(ws, "post-commit", &[]);
    }
    collect_in_background(ws);
    Ok(out)
}

/// Starts `hb gc --auto` on its own when the store has grown, like Git does
/// after a commit. Nobody waits for it, and failing here changes nothing.
fn collect_in_background(ws: &Workspace) {
    if !ws.store.needs_gc() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .args(["gc", "--auto"])
            .current_dir(&ws.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// One hunk offered by `hb commit -p`: which lines of the old file it
/// replaces, and with what.
struct Hunk {
    old_start: usize,
    old_len: usize,
    new_lines: Vec<String>,
}

/// What `hb commit -p` keeps of each file: the hunks answered yes.
type Chosen = BTreeMap<String, Vec<Hunk>>;

fn file_hunks(tx: &Tx<'_>, old: &str, new: &str) -> (Vec<Hunk>, Vec<String>) {
    let _ = tx;
    let patch = diffy::DiffOptions::new().create_patch(old, new);
    let mut hunks = Vec::new();
    let mut shown = Vec::new();
    for hunk in patch.hunks() {
        let mut new_lines = Vec::new();
        let mut text = String::new();
        for line in hunk.lines() {
            match line {
                diffy::Line::Context(l) => {
                    new_lines.push((*l).to_owned());
                    text.push(' ');
                    text.push_str(l);
                }
                diffy::Line::Insert(l) => {
                    new_lines.push((*l).to_owned());
                    text.push('+');
                    text.push_str(l);
                }
                diffy::Line::Delete(l) => {
                    text.push('-');
                    text.push_str(l);
                }
            }
        }
        hunks.push(Hunk {
            old_start: hunk.old_range().start().saturating_sub(1),
            old_len: hunk.old_range().len(),
            new_lines,
        });
        shown.push(text);
    }
    (hunks, shown)
}

/// Asks about each hunk on standard input, one answer per line: `y`, `n`, `a`
/// (this and the rest of the file) or `q` (stop). Answers can come from a
/// pipe, so agents can use it too.
fn choose_hunks(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    spec: &PathSpec,
) -> Result<Chosen> {
    use std::io::BufRead;
    let files = ws.read(actor, prov, |tx| {
        let wc = tx.wc_id()?;
        let commit = tx.commit(&wc)?;
        let (base, _) = tx.parents_tree(&commit.parents)?;
        let tree = tx.tree(&commit.tree)?;
        let mut out = Vec::new();
        for (path, _) in hebra_core::diff::changed_paths(&base, &tree) {
            if !spec.matches(&path) {
                continue;
            }
            let read = |t: &Tree| -> Coded<String> {
                match t.files.get(&path) {
                    Some(e) => {
                        Ok(String::from_utf8_lossy(&tx.store.read("blob", &e.blob)?).into_owned())
                    }
                    None => Ok(String::new()),
                }
            };
            let (old, new) = (read(&base)?, read(&tree)?);
            let (hunks, shown) = file_hunks(tx, &old, &new);
            out.push((path, hunks, shown));
        }
        Ok(out)
    })?;
    let stdin = std::io::stdin();
    let mut answers = stdin.lock().lines();
    let mut chosen = Chosen::new();
    'files: for (path, hunks, shown) in files {
        let mut take_rest = false;
        for (hunk, text) in hunks.into_iter().zip(shown) {
            if !take_rest {
                eprint!("--- {path}\n{text}Take this hunk? [y,n,a,q] ");
                let answer = answers
                    .next()
                    .transpose()?
                    .unwrap_or_else(|| "q".to_owned());
                match answer.trim() {
                    "a" => take_rest = true,
                    "y" => {}
                    "q" => break 'files,
                    _ => continue,
                }
            }
            chosen.entry(path.clone()).or_default().push(hunk);
        }
    }
    Ok(chosen)
}

/// `old` with only the chosen hunks applied.
fn apply_hunks(old: &str, hunks: &[Hunk]) -> String {
    let ends_with_newline = old.is_empty() || old.ends_with('\n');
    let mut lines: Vec<String> = old.lines().map(|l| format!("{l}\n")).collect();
    // From the end, so earlier positions stay valid.
    for hunk in hunks.iter().rev() {
        let end = (hunk.old_start + hunk.old_len).min(lines.len());
        let start = hunk.old_start.min(end);
        lines.splice(start..end, hunk.new_lines.iter().cloned());
    }
    let mut out = lines.concat();
    if !ends_with_newline && out.ends_with('\n') && hunks.is_empty() {
        out.pop();
    }
    out
}

/// Makes a change on top of `@-` with the chosen hunks; the rest stays in `@`.
fn commit_hunks(
    tx: &mut Tx<'_>,
    wc: ObjectId,
    chosen: &Chosen,
    message: String,
) -> Coded<ObjectId> {
    let commit = tx.commit(&wc)?;
    let (base, _) = tx.parents_tree(&commit.parents)?;
    let current = tx.tree(&commit.tree)?;
    let mut first = base.clone();
    for (path, hunks) in chosen {
        let old = match base.files.get(path) {
            Some(e) => String::from_utf8_lossy(&tx.store.read("blob", &e.blob)?).into_owned(),
            None => String::new(),
        };
        let blob = tx
            .store
            .write("blob", apply_hunks(&old, hunks).as_bytes())?;
        let executable = current.files.get(path).is_some_and(|e| e.executable);
        first.files.insert(
            path.clone(),
            hebra_core::model::FileEntry::file(blob, executable),
        );
    }
    tx.split_tree(wc, &first, message)
}

/// Records an author and date given by hand.
fn stamp(
    tx: &mut Tx<'_>,
    id: ObjectId,
    author: Option<Actor>,
    date: Option<String>,
) -> Coded<ObjectId> {
    if author.is_none() && date.is_none() {
        return Ok(id);
    }
    tx.rewrite(id, |c| {
        if let Some(author) = author {
            c.author = author;
        }
        if let Some(date) = date {
            c.authored_at = Some(date);
        }
    })
}

/// Signs `ids`, parents first: a signature covers the parents, so each commit
/// needs the new id of the one below. What sits on top moves onto the signed
/// commits with its files as they are. Returns the new ids, in order.
fn sign_series(tx: &mut Tx<'_>, ids: &[ObjectId], signer: &sign::Signer) -> Coded<Vec<ObjectId>> {
    let mut moved = HashMap::new();
    let mut signed = Vec::with_capacity(ids.len());
    for id in ids {
        let mut commit = tx.commit(id)?;
        for parent in &mut commit.parents {
            if let Some(new) = moved.get(parent) {
                *parent = *new;
            }
        }
        commit.signature = None;
        let payload = serde_json::to_vec(&commit)?;
        let signature = signer
            .sign(&payload)
            .map_err(|e| invalid(format!("signing failed: {e:#}")))?;
        commit.signature = Some(signature);
        let new = tx.write_commit(commit)?;
        tx.replace_refs(*id, new);
        moved.insert(*id, new);
        signed.push(new);
    }
    for (old, new) in &moved {
        for child in tx.children_of(*old)? {
            if !moved.contains_key(&child) {
                tx.rewrite(child, |c| {
                    for parent in &mut c.parents {
                        if parent == old {
                            *parent = *new;
                        }
                    }
                })?;
            }
        }
    }
    Ok(signed)
}

/// Signs `id` if there is a signer, and returns what it became.
pub(crate) fn sign_one(
    tx: &mut Tx<'_>,
    id: ObjectId,
    signer: Option<&sign::Signer>,
) -> Coded<ObjectId> {
    match signer {
        Some(signer) => Ok(sign_series(tx, &[id], signer)?.pop().unwrap_or(id)),
        None => Ok(id),
    }
}

/// Signs what a rebase made: the commits in `after`, parents first, that are
/// not in `before`. One that did not move keeps its id, and any signature it
/// has. A working copy is left out: it is still being written.
pub(crate) fn sign_rebased(
    tx: &mut Tx<'_>,
    before: &[ObjectId],
    mut after: Vec<ObjectId>,
    signer: &sign::Signer,
) -> Coded<()> {
    after.retain(|id| !before.contains(id) && !tx.view.workspaces.values().any(|w| w.wc == *id));
    sign_series(tx, &after, signer).map(drop)
}

// --------------------------------------------------------- moving around

/// Goes to `target`, on `bookmark` if given. Uncommitted work comes along, as
/// in Git; a change with a description stays where it is, since it is work
/// someone named.
fn go_to(tx: &mut Tx<'_>, target: ObjectId, bookmark: Option<String>) -> Coded<ObjectId> {
    let wc = tx.wc_id()?;
    let commit = tx.commit(&wc)?;
    let new = if commit.parents == [target] {
        wc
    } else if !tx.is_empty_change(&wc)? && commit.description.is_empty() {
        let (tree, _, conflicts) = tx.replayed_tree(wc, &[target], None)?;
        tx.rewrite(wc, |c| {
            c.parents = vec![target];
            c.tree = tree;
            c.conflicts = conflicts;
        })?
    } else {
        let tree = tx.commit(&target)?.tree;
        let id = tx.new_commit(vec![target], tree, String::new(), Vec::new())?;
        tx.switch_wc(id)?;
        id
    };
    tx.set_current_bookmark(bookmark);
    Ok(new)
}

fn valid_bookmark(name: &str) -> Coded<()> {
    if name.is_empty()
        || name.starts_with(['-', '/'])
        || name.ends_with('/')
        || name.contains("..")
        || name.contains('@')
        || name
            .chars()
            .any(|c| c.is_whitespace() || "~^:?*[\\".contains(c))
    {
        return Err(invalid(format!("`{name}` is not a valid branch name")));
    }
    Ok(())
}

/// Moves onto `target`, staying on `bookmark`: what a pull does when the
/// bookmark moved.
pub(crate) fn follow(tx: &mut Tx<'_>, target: ObjectId, bookmark: &str) -> Coded<()> {
    go_to(tx, target, Some(bookmark.to_owned()))?;
    Ok(())
}

/// Merges `other` into the working copy, as a pull that diverged does.
pub(crate) fn merge_into_wc(tx: &mut Tx<'_>, other: ObjectId) -> Coded<()> {
    let wc = tx.wc_id()?;
    let parents = if tx.is_empty_change(&wc)? {
        tx.commit(&wc)?.parents
    } else {
        vec![wc]
    };
    let mut parents = parents;
    parents.push(other);
    let (tree, conflicts) = tx.parents_tree(&parents)?;
    let tree_id = tx.store.write_tree(&tree)?;
    let id = tx.new_commit(parents, tree_id, "merge".to_owned(), conflicts)?;
    tx.switch_wc(id)
}

pub fn switch(
    tx: &mut Tx<'_>,
    target: Option<&str>,
    create: Option<&str>,
    detach: bool,
) -> Coded<Output> {
    // `-` is where you were before, as in `git switch -`.
    let previous_key = format!("workspace.{}.previous", tx.ws_name);
    let before = tx.current_bookmark();
    let remembered = tx.config_value(&previous_key)?;
    let target = match target {
        Some("-") => Some(
            remembered
                .as_deref()
                .ok_or_else(|| invalid("there is no previous branch"))?,
        ),
        other => other,
    };
    let (id, bookmark) = if let Some(name) = create {
        valid_bookmark(name)?;
        if tx.view.bookmarks.contains_key(name) {
            return Err(invalid(format!("branch `{name}` already exists")));
        }
        let start = tx.resolve(target.unwrap_or("@-"))?;
        tx.view.bookmarks.insert(name.to_owned(), start);
        (start, Some(name.to_owned()))
    } else {
        let name = target.ok_or_else(|| invalid("say where to go"))?;
        let id = tx.resolve(name)?;
        let bookmark = (!detach && tx.view.bookmarks.contains_key(name)).then(|| name.to_owned());
        (id, bookmark)
    };
    let wc = go_to(tx, id, bookmark.clone())?;
    if let Some(name) = before {
        tx.config_store(&previous_key, &name)?;
    }
    let commit = tx.commit(&wc)?;
    let text = match &bookmark {
        Some(name) => format!(
            "On branch {name}\nWorking copy is now {}\n",
            short_line(&wc, &commit)
        ),
        None => format!("Working copy is now {}\n", short_line(&wc, &commit)),
    };
    Ok(Output::new(
        json!({ "bookmark": bookmark, "working_copy": commit_json(&wc, &commit) }),
        text,
    ))
}

pub fn restore(tx: &mut Tx<'_>, paths: &[String], source: &str) -> Coded<Output> {
    let spec = PathSpec::new(paths)?;
    let from = tx.resolve(source)?;
    let from_tree = tx.commit_tree(&from)?;
    let wc = tx.wc_id()?;
    let current = tx.commit_tree(&wc)?;
    let restored = take_paths(&current, &from_tree, &spec);
    let changed = hebra_core::diff::changed_paths(&current, &restored);
    tx.set_wc_tree(&restored, Vec::new())?;
    let names: Vec<&String> = changed.iter().map(|(p, _)| p).collect();
    Ok(Output::new(
        json!({ "restored": names, "source": from.hex() }),
        format!("Restored {} file(s) from {}\n", names.len(), from.short()),
    ))
}

pub fn reset(tx: &mut Tx<'_>, revision: &str, hard: bool) -> Coded<Output> {
    let target = tx.resolve(revision)?;
    let wc = tx.reset(target, hard)?;
    let commit = tx.commit(&wc)?;
    Ok(Output::new(
        json!({ "target": target.hex(), "hard": hard, "working_copy": commit_json(&wc, &commit) }),
        format!(
            "Now at {}{}\n",
            target.short(),
            if hard { "" } else { "; files unchanged" }
        ),
    ))
}

pub fn edit(tx: &mut Tx<'_>, revision: &str) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    if id == tx.root_commit {
        return Err(invalid("the root commit cannot be modified"));
    }
    tx.switch_wc(id)?;
    tx.set_current_bookmark(None);
    let commit = tx.commit(&id)?;
    Ok(Output::new(
        commit_json(&id, &commit),
        format!("Working copy is now {}\n", short_line(&id, &commit)),
    ))
}

// ---------------------------------------------------------------- merge

pub fn merge(tx: &mut Tx<'_>, args: &MergeArgs) -> Coded<Output> {
    let wc = tx.wc_id()?;
    let empty_wc = tx.is_empty_change(&wc)?;
    let mut parents = if empty_wc {
        tx.commit(&wc)?.parents
    } else {
        vec![wc]
    };
    let original = parents.clone();
    let mut names = Vec::new();
    for rev in &args.revisions {
        let id = tx.resolve(rev)?;
        if !parents.contains(&id) {
            parents.push(id);
        }
        let c = tx.commit(&id)?;
        names.push(format!(
            "{} ({})",
            c.change_id.short(),
            if c.description.is_empty() {
                "(no description)"
            } else {
                c.description.lines().next().unwrap_or("")
            }
        ));
    }
    let simplified = tx.simplify_parents(parents.clone())?;
    if simplified == original {
        return Err(invalid(
            "nothing to merge: the revisions are already included",
        ));
    }
    let fast_forward = simplified.len() == 1 && empty_wc;
    if args.ff_only && !fast_forward {
        return Err(invalid("not possible to fast-forward"));
    }
    if args.squash {
        return merge_squash(tx, &simplified);
    }
    let parents = if args.no_ff { parents } else { simplified };
    let message = args
        .message
        .clone()
        .unwrap_or_else(|| format!("merge {}", names.join(", ")));
    let (mut tree, mut conflicts) = tx.parents_tree(&parents)?;
    if args.strategy.as_deref() == Some("ours") {
        tree = tx.commit_tree(&parents[0])?;
        conflicts.clear();
    } else if let Some(option) = args.strategy_option.as_deref() {
        let side = match option {
            "ours" => Side::Ours,
            "theirs" => Side::Theirs,
            other => return Err(invalid(format!("unknown strategy option `{other}`"))),
        };
        conflicts = settle(tx, &mut tree, &conflicts, side)?;
    }
    let tree_id = tx.store.write_tree(&tree)?;
    let id = tx.new_commit(parents.clone(), tree_id, message, conflicts)?;
    tx.switch_wc(id)?;
    if fast_forward && let [only] = parents[..] {
        advance_bookmark(tx, only)?;
    }
    let commit = tx.commit(&id)?;
    let mut text = format!("Merged into {}\n", short_line(&id, &commit));
    if commit.conflicts.is_empty() {
        let _ = writeln!(text, "No conflicts.");
    } else {
        let _ = writeln!(text, "Conflicts (recorded, not blocking):");
        for c in &commit.conflicts {
            let _ = writeln!(text, "  ! {}", c.path);
        }
        let _ = writeln!(
            text,
            "Edit the files to resolve; markers are detected automatically."
        );
    }
    Ok(Output::new(commit_json(&id, &commit), text))
}

/// `git merge --squash`: the files arrive, the history does not.
fn merge_squash(tx: &mut Tx<'_>, parents: &[ObjectId]) -> Coded<Output> {
    let (tree, conflicts) = tx.parents_tree(parents)?;
    let id = tx.set_wc_tree(&tree, conflicts)?;
    let commit = tx.commit(&id)?;
    Ok(Output::new(
        commit_json(&id, &commit),
        format!(
            "Squashed into the working copy {}\n",
            short_line(&id, &commit)
        ),
    ))
}

/// Resolves content conflicts in `tree` by taking one side of each region.
/// Returns the conflicts it could not settle.
fn settle(
    tx: &mut Tx<'_>,
    tree: &mut Tree,
    conflicts: &[Conflict],
    side: Side,
) -> Coded<Vec<Conflict>> {
    let mut left = Vec::new();
    for conflict in conflicts {
        if conflict.kind != ConflictKind::Content {
            left.push(conflict.clone());
            continue;
        }
        let Some(entry) = tree.files.get_mut(&conflict.path) else {
            continue;
        };
        let data = tx.store.read("blob", &entry.blob)?;
        entry.blob = tx
            .store
            .write("blob", &merge::settle_markers(&data, side))?;
    }
    Ok(left)
}

pub fn resolve(tx: &mut Tx<'_>, paths: &[String], ours: bool, theirs: bool) -> Coded<Output> {
    let wc = tx.wc_id()?;
    let commit = tx.commit(&wc)?;
    let before = commit.conflicts.clone();
    let chosen: Vec<Conflict> = before
        .iter()
        .filter(|c| paths.is_empty() || paths.contains(&c.path))
        .cloned()
        .collect();
    let remaining: Vec<Conflict> = before
        .iter()
        .filter(|c| !chosen.contains(c))
        .cloned()
        .collect();
    let new = if ours || theirs {
        let side = if ours { Side::Ours } else { Side::Theirs };
        let mut tree = tx.tree(&commit.tree)?;
        settle(tx, &mut tree, &chosen, side)?;
        tx.set_wc_tree(&tree, remaining)?
    } else {
        tx.rewrite(wc, |c| c.conflicts = remaining)?
    };
    let commit = tx.commit(&new)?;
    let mut text = format!(
        "Marked {} conflict(s) as resolved.\n",
        before.len() - commit.conflicts.len()
    );
    for c in &commit.conflicts {
        let _ = writeln!(text, "  ! {}", c.path);
    }
    Ok(Output::new(
        json!({ "resolved": before.len() - commit.conflicts.len(), "remaining": commit.conflicts }),
        text,
    ))
}

// ------------------------------------------------------- copying changes

/// Puts `tree` on top of the working copy as a finished change.
fn land(
    tx: &mut Tx<'_>,
    tree: &Tree,
    conflicts: Vec<Conflict>,
    make: impl FnOnce(&mut Commit),
) -> Coded<ObjectId> {
    let wc = tx.wc_id()?;
    let empty = tx.is_empty_change(&wc)?;
    let tree_id = tx.store.write_tree(tree)?;
    let done = if empty {
        tx.rewrite(wc, |c| {
            c.tree = tree_id;
            c.conflicts = conflicts;
            make(c);
        })?
    } else {
        let id = tx.new_commit(vec![wc], tree_id, String::new(), conflicts)?;
        tx.switch_wc(id)?;
        tx.rewrite(id, make)?
    };
    let next = tx.new_commit(vec![done], tree_id, String::new(), Vec::new())?;
    tx.switch_wc(next)?;
    advance_bookmark(tx, done)?;
    Ok(done)
}

pub fn cherry_pick(
    tx: &mut Tx<'_>,
    revisions: &[String],
    no_commit: bool,
    record_origin: bool,
    mainline: Option<usize>,
    signer: Option<&sign::Signer>,
) -> Coded<Output> {
    let mut picked = Vec::new();
    let mut text = String::new();
    for spec in revisions {
        for id in expand_range(tx, spec)? {
            let source = tx.commit(&id)?;
            let wc = tx.wc_id()?;
            let (_, tree, conflicts) = tx.replayed_tree(id, &[wc], mainline)?;
            if no_commit {
                tx.set_wc_tree(&tree, conflicts)?;
                picked.push(json!({ "from": id.hex() }));
                continue;
            }
            let mut description = source.description.clone();
            if record_origin {
                let _ = write!(description, "\n\n(cherry picked from commit {})", id.hex());
            }
            let (author, when) = (
                source.author.clone(),
                source
                    .authored_at
                    .clone()
                    .or(Some(source.timestamp.clone())),
            );
            let provenance = source.provenance.clone();
            let done = land(tx, &tree, conflicts, |c| {
                c.description = description;
                c.author = author;
                c.authored_at = when;
                c.provenance = provenance;
            })?;
            let done = sign_one(tx, done, signer)?;
            let commit = tx.commit(&done)?;
            let _ = writeln!(text, "Picked {}", short_line(&done, &commit));
            for c in &commit.conflicts {
                let _ = writeln!(text, "  ! {} (recorded, not blocking)", c.path);
            }
            picked.push(commit_json(&done, &commit));
        }
    }
    Ok(Output::new(json!({ "picked": picked }), text))
}

pub fn revert(
    tx: &mut Tx<'_>,
    revisions: &[String],
    no_commit: bool,
    mainline: Option<usize>,
    signer: Option<&sign::Signer>,
) -> Coded<Output> {
    let mut done_list = Vec::new();
    let mut text = String::new();
    for spec in revisions {
        let mut ids = expand_range(tx, spec)?;
        ids.reverse();
        for id in ids {
            let source = tx.commit(&id)?;
            let parent = match (mainline, source.parents.len()) {
                (Some(n), _) => source.parents.get(n.saturating_sub(1)).copied(),
                (None, 1) => source.parents.first().copied(),
                (None, _) => {
                    return Err(invalid(
                        "this commit is a merge: choose a side with --mainline",
                    ));
                }
            }
            .ok_or_else(|| invalid("that parent does not exist"))?;
            let base = tx.tree(&source.tree)?;
            let wc = tx.wc_id()?;
            let ours = tx.commit_tree(&wc)?;
            let theirs = tx.commit_tree(&parent)?;
            let merged = hebra_core::merge::merge_trees(tx.store, &base, &ours, &theirs)?;
            if no_commit {
                tx.set_wc_tree(&merged.tree, merged.conflicts)?;
                continue;
            }
            let title = subject(&source.description);
            let description = format!("Revert \"{title}\"\n\nThis reverts commit {}.", id.hex());
            let done = land(tx, &merged.tree, merged.conflicts, |c| {
                c.description = description;
            })?;
            let done = sign_one(tx, done, signer)?;
            let commit = tx.commit(&done)?;
            let _ = writeln!(text, "Reverted with {}", short_line(&done, &commit));
            done_list.push(commit_json(&done, &commit));
        }
    }
    Ok(Output::new(json!({ "reverted": done_list }), text))
}

/// `A..B` as a list, parents first; anything else is one revision.
pub(crate) fn expand_range(tx: &mut Tx<'_>, spec: &str) -> Coded<Vec<ObjectId>> {
    if let Some((from, to)) = spec.split_once("..") {
        let to = tx.resolve(if to.is_empty() { "@-" } else { to })?;
        let from = tx.resolve(from)?;
        return tx.range(Some(from), to);
    }
    Ok(vec![tx.resolve(spec)?])
}

// ---------------------------------------------------------------- rebase

fn parse_todo(tx: &mut Tx<'_>, text: &str) -> Coded<Vec<Step>> {
    let mut steps = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, char::is_whitespace);
        let action = parts.next().unwrap_or_default();
        let rev = parts
            .next()
            .ok_or_else(|| invalid(format!("no revision in `{line}`")))?;
        let rest = parts.next().unwrap_or("").trim();
        let id = tx.resolve(rev)?;
        steps.push(match action {
            "pick" | "p" => Step::Pick(id),
            "reword" | "r" => Step::Reword(id, rest.to_owned()),
            "squash" | "s" => Step::Squash(id),
            "fixup" | "f" => Step::Fixup(id),
            "drop" | "d" => Step::Drop(id),
            other => return Err(invalid(format!("unknown step `{other}`"))),
        });
    }
    Ok(steps)
}

/// Moves `fixup!` and `squash!` changes right after their targets.
fn autosquash(tx: &mut Tx<'_>, commits: &[ObjectId]) -> Coded<Vec<Step>> {
    let mut titles: Vec<(ObjectId, String)> = Vec::new();
    for id in commits {
        titles.push((*id, subject(&tx.commit(id)?.description)));
    }
    let mut steps: Vec<(ObjectId, Step)> = Vec::new();
    for (id, title) in &titles {
        let folded = title
            .strip_prefix("fixup! ")
            .map(|t| (t, Step::Fixup(*id)))
            .or_else(|| {
                title
                    .strip_prefix("squash! ")
                    .map(|t| (t, Step::Squash(*id)))
            });
        match folded {
            Some((target, step)) => {
                let spot = steps.iter().rposition(|(owner, _)| {
                    titles
                        .iter()
                        .find(|(i, _)| i == owner)
                        .is_some_and(|(_, t)| t == target)
                });
                match spot {
                    Some(i) => {
                        let mut at = i + 1;
                        while at < steps.len()
                            && matches!(steps[at].1, Step::Fixup(_) | Step::Squash(_))
                        {
                            at += 1;
                        }
                        let owner = steps[i].0;
                        steps.insert(at, (owner, step));
                    }
                    None => steps.push((*id, Step::Pick(*id))),
                }
            }
            None => steps.push((*id, Step::Pick(*id))),
        }
    }
    Ok(steps.into_iter().map(|(_, s)| s).collect())
}

fn default_todo(tx: &mut Tx<'_>, commits: &[ObjectId]) -> Coded<String> {
    let mut text = String::from(
        "# pick, reword <new text>, squash, fixup, drop. Reorder lines to reorder changes.\n",
    );
    for id in commits {
        let c = tx.commit(id)?;
        let _ = writeln!(text, "pick {}  # {}", id.short(), subject(&c.description));
    }
    Ok(text)
}

/// What to move and where. The todo text, if any, comes from outside the
/// transaction because an editor must not run while one is open.
pub struct RebasePlan {
    pub todo: Option<String>,
}

/// Runs edited steps. Steps can be reordered, so what pointed at the end of
/// the series (its branch, the working copy on top) goes to the new end.
/// Returns how many steps ran and the series as it ended, parents first.
fn apply_series(
    tx: &mut Tx<'_>,
    steps: &[Step],
    dest: ObjectId,
    commits: &[ObjectId],
) -> Coded<(usize, Vec<ObjectId>)> {
    let last = commits.last().copied();
    let marks: Vec<String> = tx
        .view
        .bookmarks
        .iter()
        .filter(|(_, id)| Some(**id) == last)
        .map(|(name, _)| name.clone())
        .collect();
    let on_top = match last {
        Some(id) => tx.children_of(id)?,
        None => Vec::new(),
    };
    let tip = tx.apply_steps(steps, dest)?;
    for name in marks {
        tx.view.bookmarks.insert(name, tip);
    }
    if let Some(id) = last {
        for child in on_top {
            if !commits.contains(&child) {
                tx.rebase_child(child, id, tip)?;
            }
        }
    }
    Ok((steps.len(), tx.range(Some(dest), tip)?))
}

pub fn rebase(
    tx: &mut Tx<'_>,
    args: &RebaseArgs,
    plan: &RebasePlan,
    signer: Option<&sign::Signer>,
) -> Coded<Output> {
    let dest_name = args
        .destination
        .as_deref()
        .or(args.upstream.as_deref())
        .map(str::to_owned)
        .or_else(|| {
            // Like `git rebase` with an upstream configured.
            tx.current_bookmark().and_then(|b| {
                tx.view
                    .remotes
                    .get("origin")
                    .filter(|r| r.contains_key(&b))
                    .map(|_| format!("{b}@origin"))
            })
        })
        .ok_or_else(|| invalid("say where to rebase onto (`hb rebase main`)"))?;
    let dest = tx.resolve(&dest_name)?;
    let commits: Vec<ObjectId> = if let Some(source) = &args.source {
        let id = tx.resolve(source)?;
        let mut all = vec![id];
        all.extend(tx.descendants(id)?);
        all
    } else if let Some(revision) = &args.revision {
        let id = tx.resolve(revision)?;
        // Only this one: its children stay where they are, on its parent.
        let commit = tx.commit(&id)?;
        let parent = commit.parents.first().copied().unwrap_or(tx.root_commit);
        for child in tx.children_of(id)? {
            tx.rebase_child(child, id, parent)?;
        }
        vec![id]
    } else {
        let edited = plan.todo.is_some() || args.autosquash;
        let branch = args
            .branch
            .as_deref()
            .unwrap_or(if edited { "@-" } else { "@" });
        let tip = tx.resolve(branch)?;
        tx.range(Some(dest), tip)?
    };
    if commits.is_empty() {
        return Ok(Output::new(
            json!({ "moved": 0 }),
            "Already up to date.\n".to_owned(),
        ));
    }
    let (moved, after) = if let Some(todo) = &plan.todo {
        let steps = parse_todo(tx, todo)?;
        apply_series(tx, &steps, dest, &commits)?
    } else if args.autosquash {
        let steps = autosquash(tx, &commits)?;
        apply_series(tx, &steps, dest, &commits)?
    } else {
        let moved = tx.rebase(&commits, dest)?;
        let after = commits.iter().filter_map(|id| moved.get(id).copied());
        (moved.len(), after.collect())
    };
    if let Some(signer) = signer {
        sign_rebased(tx, &commits, after, signer)?;
    }
    let mut text = format!("Rebased {moved} change(s) onto {}\n", dest.short());
    let conflicted = wc_conflicts(tx)?;
    for path in &conflicted {
        let _ = writeln!(text, "  ! {path} (recorded, not blocking)");
    }
    Ok(Output::new(
        json!({ "moved": moved, "destination": dest.hex(), "conflicts": conflicted }),
        text,
    ))
}

fn wc_conflicts(tx: &mut Tx<'_>) -> Coded<Vec<String>> {
    let wc = tx.wc_id()?;
    Ok(tx
        .commit(&wc)?
        .conflicts
        .into_iter()
        .map(|c| c.path)
        .collect())
}

pub fn rebase_todo_needed(args: &RebaseArgs) -> bool {
    args.interactive && args.todo.is_none()
}

/// The steps as the user left them, from a file or the editor.
pub fn rebase_plan(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    args: &RebaseArgs,
) -> Result<RebasePlan> {
    if args.abort {
        bail!("nothing to abort: a rebase never stops half-way. `hb undo` takes it back");
    }
    if let Some(file) = &args.todo {
        let text =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        return Ok(RebasePlan { todo: Some(text) });
    }
    if !rebase_todo_needed(args) {
        return Ok(RebasePlan { todo: None });
    }
    let todo = ws.read(actor, prov, |tx| {
        let dest = args
            .destination
            .as_deref()
            .or(args.upstream.as_deref())
            .ok_or_else(|| invalid("say where to rebase onto (`hb rebase -i main`)"))?;
        let dest = tx.resolve(dest)?;
        let tip = tx.resolve(args.branch.as_deref().unwrap_or("@-"))?;
        let commits = tx.range(Some(dest), tip)?;
        default_todo(tx, &commits)
    })?;
    let edited = edit_text(ws, "rebase-todo", &todo)?;
    Ok(RebasePlan { todo: Some(edited) })
}

pub fn squash(
    tx: &mut Tx<'_>,
    from: &str,
    into: Option<&str>,
    message: Option<String>,
    paths: &[String],
) -> Coded<Output> {
    let source = tx.resolve(from)?;
    let into = match into {
        Some(rev) => tx.resolve(rev)?,
        None => tx
            .commit(&source)?
            .parents
            .first()
            .copied()
            .filter(|p| *p != tx.root_commit)
            .ok_or_else(|| invalid("there is no parent to squash into"))?,
    };
    let spec = PathSpec::new(paths)?;
    let new = tx.squash(source, into, &spec, message)?;
    let commit = tx.commit(&new)?;
    Ok(Output::new(
        commit_json(&new, &commit),
        format!("Squashed into {}\n", short_line(&new, &commit)),
    ))
}

pub fn split(tx: &mut Tx<'_>, revision: &str, message: String, paths: &[String]) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    let spec = PathSpec::new(paths)?;
    let first = tx.split(id, &spec, message)?;
    let commit = tx.commit(&first)?;
    Ok(Output::new(
        commit_json(&first, &commit),
        format!("Split off {}\n", short_line(&first, &commit)),
    ))
}

pub fn abandon(tx: &mut Tx<'_>, revisions: &[String]) -> Coded<Output> {
    let mut gone = Vec::new();
    for rev in revisions {
        let id = tx.resolve(rev)?;
        tx.abandon(id)?;
        gone.push(id.hex());
    }
    Ok(Output::new(
        json!({ "abandoned": gone }),
        format!(
            "Abandoned {} change(s). `hb undo` brings them back.\n",
            gone.len()
        ),
    ))
}

/// Rewrites a history keeping only some paths.
pub fn filter(
    tx: &mut Tx<'_>,
    remove: &[String],
    keep: &[String],
    revision: &str,
) -> Coded<Output> {
    if remove.is_empty() && keep.is_empty() {
        return Err(invalid("say what to --remove or --keep"));
    }
    let (remove, keep) = (PathSpec::new(remove)?, PathSpec::new(keep)?);
    let filtered = |tree: Tree, remove: &PathSpec, keep: &PathSpec, drop: bool, kept: bool| {
        let mut tree = tree;
        tree.files
            .retain(|path, _| (!drop || !remove.matches(path)) && (!kept || keep.matches(path)));
        tree
    };
    let (drop_paths, keep_only) = (!remove.is_all(), !keep.is_all());
    let tip = tx.resolve(revision)?;
    let commits = tx.range(None, tip)?;
    let mut moved: BTreeMap<ObjectId, ObjectId> = BTreeMap::new();
    let mut last = None;
    for id in &commits {
        let mut commit = tx.commit(id)?;
        let tree = filtered(
            tx.tree(&commit.tree)?,
            &remove,
            &keep,
            drop_paths,
            keep_only,
        );
        commit.tree = tx.store.write_tree(&tree)?;
        commit.parents = commit
            .parents
            .iter()
            .map(|p| moved.get(p).copied().unwrap_or(*p))
            .collect();
        commit.predecessors = vec![*id];
        commit.signature = None;
        let new = tx.write_commit(commit)?;
        moved.insert(*id, new);
        tx.replace_refs(*id, new);
        last = Some(new);
    }
    // Whatever was on top comes along.
    if let Some(new) = last {
        for child in tx.children_of(tip)? {
            tx.rebase_child(child, tip, new)?;
        }
    }
    Ok(Output::new(
        json!({ "rewritten": moved.len() }),
        format!(
            "Rewrote {} change(s). `hb undo` takes it back.\n",
            moved.len()
        ),
    ))
}

// -------------------------------------------------------------- branches

fn list_refs(tx: &mut Tx<'_>, args: &BranchArgs) -> Coded<Output> {
    let current = tx.current_bookmark();
    if args.show_current {
        let name = current.clone().unwrap_or_default();
        return Ok(Output::new(json!({ "current": name }), format!("{name}\n")));
    }
    let mut rows: Vec<(String, ObjectId, bool)> = Vec::new();
    if !args.remotes {
        for (name, id) in &tx.view.bookmarks {
            rows.push((name.clone(), *id, false));
        }
    }
    if args.remotes || args.all {
        for (remote, marks) in &tx.view.remotes {
            for (name, id) in marks {
                rows.push((format!("{remote}/{name}"), *id, true));
            }
        }
    }
    let contains = args
        .contains
        .as_deref()
        .map(|r| tx.resolve(r))
        .transpose()?;
    let merged = args.merged.as_deref().map(|r| tx.resolve(r)).transpose()?;
    let no_merged = args
        .no_merged
        .as_deref()
        .map(|r| tx.resolve(r))
        .transpose()?;
    let mut text = String::new();
    let mut out = Vec::new();
    for (name, id, remote) in rows {
        if let Some(rev) = contains
            && !tx.is_ancestor(rev, id)?
        {
            continue;
        }
        if let Some(rev) = merged
            && !tx.is_ancestor(id, rev)?
        {
            continue;
        }
        if let Some(rev) = no_merged
            && tx.is_ancestor(id, rev)?
        {
            continue;
        }
        let c = tx.commit(&id)?;
        let marker = if current.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            " "
        };
        let _ = writeln!(text, "{marker} {name}  {}", short_line(&id, &c));
        out.push(json!({ "name": name, "current": marker == "*", "remote": remote, "commit": commit_json(&id, &c) }));
    }
    Ok(Output::new(json!({ "branches": out }), text))
}

pub fn branch(tx: &mut Tx<'_>, args: &BranchArgs) -> Coded<Output> {
    if args.delete {
        let name = args
            .name
            .as_deref()
            .ok_or_else(|| invalid("say which branch"))?;
        tx.view
            .bookmarks
            .remove(name)
            .ok_or_else(|| Error::RevisionNotFound(name.to_owned()))?;
        tx.view.bookmark_conflicts.remove(name);
        return Ok(Output::new(
            json!({ "deleted": name }),
            format!("Deleted branch {name}\n"),
        ));
    }
    if args.rename {
        let (old, new) = match (&args.name, &args.start) {
            (Some(old), Some(new)) => (old.clone(), new.clone()),
            (Some(new), None) => (
                tx.current_bookmark()
                    .ok_or_else(|| invalid("not on a branch: say which one to rename"))?,
                new.clone(),
            ),
            _ => return Err(invalid("say the new name")),
        };
        return bookmark_rename(tx, &old, &new);
    }
    let Some(name) = &args.name else {
        return list_refs(tx, args);
    };
    valid_bookmark(name)?;
    if tx.view.bookmarks.contains_key(name) && !args.force {
        return Err(invalid(format!("branch `{name}` already exists")));
    }
    let id = tx.resolve(args.start.as_deref().unwrap_or("@-"))?;
    tx.view.bookmarks.insert(name.clone(), id);
    tx.view.bookmark_conflicts.remove(name);
    let c = tx.commit(&id)?;
    Ok(Output::new(
        json!({ "bookmark": name, "commit": commit_json(&id, &c) }),
        format!("Branch {name} -> {}\n", short_line(&id, &c)),
    ))
}

pub fn bookmark_rename(tx: &mut Tx<'_>, old: &str, new: &str) -> Coded<Output> {
    valid_bookmark(new)?;
    if tx.view.bookmarks.contains_key(new) {
        return Err(invalid(format!("branch `{new}` already exists")));
    }
    let id = tx
        .view
        .bookmarks
        .remove(old)
        .ok_or_else(|| Error::RevisionNotFound(old.to_owned()))?;
    tx.view.bookmarks.insert(new.to_owned(), id);
    for ws in tx.view.workspaces.values_mut() {
        if ws.bookmark.as_deref() == Some(old) {
            ws.bookmark = Some(new.to_owned());
        }
    }
    Ok(Output::new(
        json!({ "renamed": old, "to": new }),
        format!("Renamed {old} to {new}\n"),
    ))
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    let Ok(glob) = globset_like(pattern) else {
        return name.contains(pattern);
    };
    glob.is_match(name)
}

fn globset_like(pattern: &str) -> Result<regex::Regex, regex::Error> {
    let mut text = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => text.push_str(".*"),
            '?' => text.push('.'),
            other => text.push_str(&regex::escape(&other.to_string())),
        }
    }
    text.push('$');
    regex::Regex::new(&text)
}

fn tag_list(tx: &mut Tx<'_>, pattern: Option<&str>, contains: Option<&str>) -> Coded<Output> {
    let filter = contains.map(|r| tx.resolve(r)).transpose()?;
    let tags: Vec<(String, Tag)> = tx
        .view
        .tags
        .iter()
        .filter(|(n, _)| pattern.is_none_or(|p| matches_pattern(n, p)))
        .map(|(n, t)| (n.clone(), t.clone()))
        .collect();
    let mut text = String::new();
    let mut rows = Vec::new();
    for (name, tag) in tags {
        if let Some(rev) = filter
            && !tx.is_ancestor(rev, tag.target)?
        {
            continue;
        }
        let _ = writeln!(text, "{name}");
        rows.push(json!({ "name": name, "tag": tag }));
    }
    Ok(Output::new(json!({ "tags": rows }), text))
}

pub fn tag(tx: &mut Tx<'_>, args: &TagArgs, signer: Option<&sign::Signer>) -> Coded<Output> {
    if args.delete {
        let name = args
            .name
            .as_deref()
            .ok_or_else(|| invalid("say which tag"))?;
        tx.view
            .tags
            .remove(name)
            .ok_or_else(|| Error::RevisionNotFound(name.to_owned()))?;
        return Ok(Output::new(
            json!({ "deleted": name }),
            format!("Deleted tag {name}\n"),
        ));
    }
    let Some(name) = args.name.as_deref().filter(|_| !args.list) else {
        return tag_list(tx, args.name.as_deref(), args.contains.as_deref());
    };
    valid_bookmark(name)?;
    if tx.view.tags.contains_key(name) && !args.force {
        return Err(invalid(format!("tag `{name}` already exists")));
    }
    let target = tx.resolve(args.revision.as_deref().unwrap_or("@-"))?;
    let annotated = args.annotate || args.message.is_some() || args.sign;
    let mut tag = Tag {
        target,
        message: annotated.then(|| args.message.clone().unwrap_or_default()),
        tagger: annotated.then(|| tx.actor.clone()),
        timestamp: annotated.then(now_timestamp),
        signature: None,
    };
    if let Some(signer) = signer {
        let payload = serde_json::to_vec(&tag)?;
        tag.signature = Some(
            signer
                .sign(&payload)
                .map_err(|e| invalid(format!("signing failed: {e:#}")))?,
        );
    }
    tx.view.tags.insert(name.to_owned(), tag.clone());
    Ok(Output::new(
        json!({ "name": name, "tag": tag }),
        format!("Tag {name} -> {}\n", target.short()),
    ))
}

/// `git describe`: the nearest tag, with how far away it is.
pub fn describe_tag(tx: &mut Tx<'_>, revision: &str, always: bool) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    let mut best: Option<(usize, String)> = None;
    let tags: Vec<(String, ObjectId)> = tx
        .view
        .tags
        .iter()
        .map(|(n, t)| (n.clone(), t.target))
        .collect();
    for (name, target) in tags {
        if tx.is_ancestor(target, id)? {
            let distance = tx.range(Some(target), id)?.len();
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, name));
            }
        }
    }
    let text = match best {
        Some((0, name)) => name,
        Some((distance, name)) => format!("{name}-{distance}-g{}", id.short()),
        None if always => id.short(),
        None => return Err(invalid("no tag can describe this revision (try --always)")),
    };
    Ok(Output::new(
        json!({ "describe": text }),
        format!("{text}\n"),
    ))
}

// ----------------------------------------------------------------- stash

fn stash_at(tx: &Tx<'_>, spec: Option<&str>) -> Coded<(usize, ObjectId)> {
    let index = match spec {
        None => 0,
        Some(text) => text
            .trim_start_matches("stash@{")
            .trim_end_matches('}')
            .parse()
            .map_err(|_| invalid(format!("`{text}` is not a stash")))?,
    };
    tx.view
        .stashes
        .get(index)
        .map(|id| (index, *id))
        .ok_or_else(|| invalid("no such stash"))
}

pub fn stash(tx: &mut Tx<'_>, cmd: &StashCommand) -> Coded<Output> {
    match cmd {
        StashCommand::Push { message, paths, .. } => {
            let spec = PathSpec::new(paths)?;
            match tx.stash_push(message.clone(), &spec)? {
                Some(id) => Ok(Output::new(
                    json!({ "stash": id.hex() }),
                    format!("Saved the working copy as {}\n", id.short()),
                )),
                None => Ok(Output::new(
                    json!({ "stash": null }),
                    "No local changes to save.\n".to_owned(),
                )),
            }
        }
        StashCommand::Pop { stash } | StashCommand::Apply { stash } => {
            let (index, id) = stash_at(tx, stash.as_deref())?;
            let conflicts = tx.stash_apply(id)?;
            if matches!(cmd, StashCommand::Pop { .. }) {
                tx.view.stashes.remove(index);
            }
            let mut text = format!("Applied stash@{{{index}}}\n");
            for c in &conflicts {
                let _ = writeln!(text, "  ! {} (recorded, not blocking)", c.path);
            }
            Ok(Output::new(json!({ "conflicts": conflicts }), text))
        }
        StashCommand::List => {
            let ids = tx.view.stashes.clone();
            let mut text = String::new();
            let mut rows = Vec::new();
            for (i, id) in ids.iter().enumerate() {
                let c = tx.commit(id)?;
                let _ = writeln!(text, "stash@{{{i}}}: {}", subject(&c.description));
                rows.push(json!({ "index": i, "commit": commit_json(id, &c) }));
            }
            Ok(Output::new(json!({ "stashes": rows }), text))
        }
        StashCommand::Show { stash } => {
            let (_, id) = stash_at(tx, stash.as_deref())?;
            let changes = tx.own_changes(&id)?;
            let mut text = String::new();
            for (path, status) in &changes {
                let _ = writeln!(text, "{} {path}", status.letter());
            }
            Ok(Output::new(
                json!({ "changes": changes.iter().map(|(p, s)| json!({ "path": p, "status": s })).collect::<Vec<_>>() }),
                text,
            ))
        }
        StashCommand::Drop { stash } => {
            let (index, id) = stash_at(tx, stash.as_deref())?;
            tx.view.stashes.remove(index);
            Ok(Output::new(
                json!({ "dropped": id.hex() }),
                format!("Dropped stash@{{{index}}}\n"),
            ))
        }
        StashCommand::Clear => {
            let n = tx.view.stashes.len();
            tx.view.stashes.clear();
            Ok(Output::new(
                json!({ "cleared": n }),
                format!("Cleared {n} stash(es).\n"),
            ))
        }
        StashCommand::Branch { name, stash } => {
            let (index, id) = stash_at(tx, stash.as_deref())?;
            let base = tx
                .commit(&id)?
                .parents
                .first()
                .copied()
                .unwrap_or(tx.root_commit);
            valid_bookmark(name)?;
            tx.view.bookmarks.insert(name.clone(), base);
            go_to(tx, base, Some(name.clone()))?;
            let conflicts = tx.stash_apply(id)?;
            tx.view.stashes.remove(index);
            Ok(Output::new(
                json!({ "bookmark": name, "conflicts": conflicts }),
                format!("On branch {name}, with the stash applied\n"),
            ))
        }
    }
}

// ---------------------------------------------------------- notes, misc

pub fn notes_add(tx: &mut Tx<'_>, revision: &str, message: &str) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    tx.note_set(&id, message)?;
    Ok(Output::new(
        json!({ "target": id.hex(), "note": message }),
        format!("Added a note to {}\n", id.short()),
    ))
}

pub fn notes_show(tx: &mut Tx<'_>, revision: &str) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    let note = tx
        .note_get(&id)?
        .ok_or_else(|| invalid(format!("{} has no note", id.short())))?;
    Ok(Output::new(
        json!({ "target": id.hex(), "note": note }),
        format!("{note}\n"),
    ))
}

pub fn notes_list(tx: &mut Tx<'_>) -> Coded<Output> {
    let notes = tx.note_list()?;
    let text: String = notes
        .iter()
        .map(|(id, body)| format!("{} {}\n", id.short(), subject(body)))
        .collect();
    let rows: Vec<Value> = notes
        .iter()
        .map(|(id, body)| json!({ "target": id.hex(), "note": body }))
        .collect();
    Ok(Output::new(json!({ "notes": rows }), text))
}

pub fn notes_remove(tx: &mut Tx<'_>, revision: &str) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    if !tx.note_remove(&id)? {
        return Err(invalid(format!("{} has no note", id.short())));
    }
    Ok(Output::new(
        json!({ "removed": id.hex() }),
        format!("Removed the note of {}\n", id.short()),
    ))
}

// ---------------------------------------------- operations on the files

pub fn add() -> Output {
    Output::new(
        json!({ "staged": false }),
        "Nothing to do: Hebra has no staging area, every file is already in the change.\n"
            .to_owned(),
    )
}

/// The files a command names, as they exist on disk.
fn matching_files(tree: &Tree, spec: &PathSpec) -> BTreeSet<String> {
    tree.files
        .keys()
        .filter(|p| spec.matches(p))
        .cloned()
        .collect()
}

pub fn rm(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    cwd: &Path,
    paths: &[String],
    cached: bool,
    recursive: bool,
) -> Result<Output> {
    let rel = tree_paths(ws, cwd, paths)?;
    let spec = PathSpec::new(&rel)?;
    let root = ws.root.clone();
    let removed = ws.read(actor, prov, |tx| {
        let wc = tx.wc_id()?;
        let tree = tx.commit_tree(&wc)?;
        Ok(matching_files(&tree, &spec))
    })?;
    if removed.is_empty() {
        bail!("no tracked file matches {}", paths.join(" "));
    }
    let is_dir_removal = removed.iter().any(|f| !rel.contains(f));
    if is_dir_removal && !recursive {
        bail!("not removing a directory without -r");
    }
    if cached {
        let ignore = root.join(hebra_core::wc::IGNORE_FILE);
        let mut lines = std::fs::read_to_string(&ignore).unwrap_or_default();
        for file in &removed {
            let _ = writeln!(lines, "/{file}");
        }
        std::fs::write(&ignore, lines)?;
    } else {
        for file in &removed {
            let full = hebra_core::wc::safe_join(&root, file)?;
            std::fs::remove_file(&full).with_context(|| format!("removing {}", full.display()))?;
            let mut dir = full.parent();
            while let Some(d) = dir {
                if d == root || std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }
    ws.snapshot(actor, prov)?;
    let text: String = removed.iter().map(|f| format!("rm {f}\n")).collect();
    Ok(Output::new(json!({ "removed": removed }), text))
}

pub fn mv(ws: &Workspace, cwd: &Path, paths: &[String], force: bool) -> Result<Output> {
    let (dest, sources) = paths
        .split_last()
        .ok_or_else(|| anyhow!("say what to move"))?;
    let dest_full = cwd.join(dest);
    let into_dir = dest_full.is_dir();
    if sources.len() > 1 && !into_dir {
        bail!("{dest} is not a directory");
    }
    let mut moved = Vec::new();
    for source in sources {
        let from = cwd.join(source);
        if !from.exists() {
            bail!("{source} does not exist");
        }
        let to = if into_dir {
            dest_full.join(
                from.file_name()
                    .ok_or_else(|| anyhow!("invalid path {source}"))?,
            )
        } else {
            dest_full.clone()
        };
        if to.exists() && !force {
            bail!("{} already exists (use -f)", to.display());
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if to.is_file() {
            std::fs::remove_file(&to)?;
        }
        std::fs::rename(&from, &to)
            .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
        moved.push(json!({ "from": source, "to": to.strip_prefix(&ws.root).unwrap_or(&to).display().to_string() }));
    }
    Ok(Output::new(
        json!({ "moved": moved }),
        format!("Moved {} path(s).\n", sources.len()),
    ))
}

pub fn clean(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    dry_run: bool,
    force: bool,
    ignored: bool,
    only_ignored: bool,
) -> Result<Output> {
    if !dry_run && !force {
        bail!("refusing to delete files without -f (or see them with -n)");
    }
    let root = ws.root.clone();
    let mut doomed: BTreeSet<String> = if only_ignored {
        BTreeSet::new()
    } else {
        ws.read(actor, prov, |tx| {
            let wc = tx.wc_id()?;
            Ok(tx
                .own_changes(&wc)?
                .into_iter()
                .filter(|(_, s)| matches!(s, hebra_core::diff::FileStatus::Added))
                .map(|(p, _)| p)
                .collect())
        })?
    };
    if ignored || only_ignored {
        doomed.extend(hebra_core::wc::ignored_files(&root)?);
    }
    if !dry_run {
        for file in &doomed {
            let full = hebra_core::wc::safe_join(&root, file)?;
            std::fs::remove_file(&full).with_context(|| format!("removing {}", full.display()))?;
            let mut dir = full.parent();
            while let Some(d) = dir {
                if d == root || std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
        ws.snapshot(actor, prov)?;
    }
    let verb = if dry_run { "Would remove" } else { "Removed" };
    let text: String = doomed.iter().map(|f| format!("{verb} {f}\n")).collect();
    Ok(Output::new(
        json!({ "removed": doomed, "dry_run": dry_run }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_parse_with_and_without_email() {
        let p = parse_person("Ana Lopez <ana@x.dev>");
        assert_eq!(p.name, "Ana Lopez");
        assert_eq!(p.email.as_deref(), Some("ana@x.dev"));
        assert_eq!(parse_person("bot").email, None);
    }

    #[test]
    fn tag_patterns_match_like_git() {
        assert!(matches_pattern("v1.2", "v1.*"));
        assert!(!matches_pattern("v2.0", "v1.*"));
        assert!(matches_pattern("v1", "v?"));
    }
}
