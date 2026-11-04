//! Remotes of every kind behind one set of commands: Hebra servers, Git
//! repositories (GitHub, GitLab, anything `git` can reach) and other Hebra
//! repositories on disk.
//!
//! A Git remote keeps a bare mirror in `<repo>/git/<remote>.git`, and the
//! table `git_map` remembers which Git commit each Hebra commit is. Commits
//! that came from Git are never exported again: new work is built on top of
//! the existing Git commits, so a push adds to history instead of rewriting
//! it, with the same commit ids everyone else has.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{BufReader, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::gitexport;
use hebra_core::gitimport;
use hebra_core::model::{Actor, Provenance, Tag};
use hebra_core::repo::{git_commit, git_sha};
use hebra_core::{ObjectId, Tx, Workspace, transport};
use serde_json::json;

use crate::cli::CloneArgs;
use crate::commands::{Output, short_line};
use crate::porcelain::{self, Coded};
use crate::sign::Signer;

/// Excluded tips passed to `git fast-export`: enough for a normal fetch, few
/// enough for any command line. Anything else re-imports to the same commits.
const MAX_EXCLUDES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hebra,
    Git,
    Local,
}

/// Whether a URL names a Git repository rather than a Hebra server.
#[must_use]
pub fn is_git_url(url: &str) -> bool {
    let url = url.trim();
    if url.ends_with(".git")
        || url.starts_with("ssh://")
        || url.starts_with("git://")
        || url.starts_with("file://")
    {
        return true;
    }
    // scp-like: `git@github.com:owner/repo`.
    if !url.contains("://")
        && let Some((host, _)) = url.split_once(':')
        && host.contains('@')
    {
        return true;
    }
    const HOSTS: [&str; 5] = [
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "codeberg.org",
        "sr.ht",
    ];
    if let Some((_, rest)) = url.split_once("://") {
        let host = rest.split('/').next().unwrap_or("");
        return HOSTS
            .iter()
            .any(|h| host == *h || host.ends_with(&format!(".{h}")));
    }
    let path = Path::new(url);
    path.join("HEAD").is_file() || path.join(".git").exists()
}

fn is_local_hebra(url: &str) -> bool {
    Path::new(url).join(hebra_core::wc::DOT_DIR).is_dir()
}

/// The kind of a URL, as `hb remote add` and `hb clone` see it.
#[must_use]
pub fn kind_of_url(url: &str, forced_git: bool) -> Kind {
    if forced_git {
        Kind::Git
    } else if is_local_hebra(url) {
        Kind::Local
    } else if is_git_url(url) {
        Kind::Git
    } else {
        Kind::Hebra
    }
}

pub fn kind(ws: &Workspace, name: &str) -> Result<Kind> {
    Ok(
        match ws.config_get(&format!("remote.{name}.kind"))?.as_deref() {
            Some("git") => Kind::Git,
            Some("local") => Kind::Local,
            _ => Kind::Hebra,
        },
    )
}

fn kind_name(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Hebra => None,
        Kind::Git => Some("git"),
        Kind::Local => Some("local"),
    }
}

/// Records a remote that is not a Hebra server.
pub fn add(ws: &Workspace, name: &str, url: &str, forced_git: bool) -> Result<Option<Output>> {
    let found = kind_of_url(url, forced_git);
    let Some(text) = kind_name(found) else {
        return Ok(None);
    };
    ws.config_set(&format!("remote.{name}.url"), url)?;
    ws.config_set(&format!("remote.{name}.kind"), text)?;
    Ok(Some(Output::new(
        json!({ "remote": name, "url": url, "kind": text }),
        format!("Added {text} remote {name} -> {url}\n"),
    )))
}

fn url_of(ws: &Workspace, name: &str) -> Result<String> {
    ws.config_get(&format!("remote.{name}.url"))?
        .ok_or_else(|| {
            anyhow!("no remote named `{name}` (add one with `hb remote add {name} <url>`)")
        })
}

// ------------------------------------------------------------------- git

fn git_command(dir: &Path) -> Command {
    use std::io::IsTerminal;
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    // Agents have no terminal: fail instead of waiting for a password.
    if !std::io::stdin().is_terminal() {
        command.env("GIT_TERMINAL_PROMPT", "0");
    }
    command
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git_command(dir)
        .args(args)
        .output()
        .context("running git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Runs git with `input` on its standard input and returns its output.
fn git_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut child = git_command(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running git (is it installed?)")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("git has no stdin"))?
        .write_all(input)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Signs again, for Git, the exported commits that are signed in Hebra. A
/// Hebra signature covers Hebra's bytes and Git checks its own commit
/// object, so the signature is made again over that object. Commits come
/// parents first; one whose parent got a new id is written again too.
/// Returns the new id of every rewritten commit.
fn sign_for_git(
    mirror: &Path,
    signer: &Signer,
    commits: &[(String, bool)],
) -> Result<HashMap<String, String>> {
    let mut rewrites: HashMap<String, String> = HashMap::new();
    for (sha, signed) in commits {
        let text = git(mirror, &["cat-file", "commit", sha])?;
        let (header, body) = text.split_once("\n\n").unwrap_or((text.as_str(), ""));
        let mut moved = false;
        let mut lines: Vec<String> = Vec::new();
        for line in header.lines() {
            match line.strip_prefix("parent ").and_then(|p| rewrites.get(p)) {
                Some(new) => {
                    lines.push(format!("parent {new}"));
                    moved = true;
                }
                None => lines.push(line.to_owned()),
            }
        }
        if !signed && !moved {
            continue;
        }
        if *signed {
            let payload = format!("{}\n\n{body}", lines.join("\n"));
            let signature = signer.sign(payload.as_bytes())?;
            lines.push(format!("gpgsig {}", signature.replace('\n', "\n ")));
        }
        let object = format!("{}\n\n{body}", lines.join("\n"));
        let new = git_with_input(
            mirror,
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            object.as_bytes(),
        )?;
        rewrites.insert(sha.clone(), String::from_utf8_lossy(&new).trim().to_owned());
    }
    Ok(rewrites)
}

fn mirror_path(ws: &Workspace, name: &str) -> PathBuf {
    ws.repo_dir.join("git").join(format!("{name}.git"))
}

fn ensure_mirror(ws: &Workspace, name: &str) -> Result<PathBuf> {
    let mirror = mirror_path(ws, name);
    if !mirror.join("HEAD").is_file() {
        std::fs::create_dir_all(&mirror)?;
        git(&mirror, &["init", "--bare", "--quiet"])?;
    }
    Ok(mirror)
}

/// A reference of the mirror: the commit it names (tags peeled) and the
/// object it points at (the tag itself, for annotated tags).
#[derive(Debug, Clone)]
struct MirrorRef {
    name: String,
    commit: String,
    object: String,
}

fn mirror_refs(mirror: &Path) -> Result<Vec<MirrorRef>> {
    let listing = git(
        mirror,
        &[
            "for-each-ref",
            "--format=%(objectname) %(*objectname) %(refname)",
        ],
    )?;
    let mut out = Vec::new();
    for line in listing.lines() {
        let mut parts: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = parts.pop() else { continue };
        if let (Some(object), Some(commit)) = (parts.first(), parts.last()) {
            out.push(MirrorRef {
                name: name.to_owned(),
                commit: (*commit).to_owned(),
                object: (*object).to_owned(),
            });
        }
    }
    Ok(out)
}

/// Cuts a mirror's history down to the commits its references name: once
/// Hebra has the history, Git only needs the tips to fetch more and to push
/// on top of them. Without this, a repository cloned from Git would keep
/// every Git object twice. History comes back if a push needs it.
fn compact_mirror(mirror: &Path) -> Result<()> {
    let mut tips: Vec<String> = mirror_refs(mirror)?.into_iter().map(|r| r.commit).collect();
    tips.sort();
    tips.dedup();
    if tips.is_empty() {
        return Ok(());
    }
    let mut text = tips.join("\n");
    text.push('\n');
    std::fs::write(mirror.join("shallow"), text)?;
    git(mirror, &["repack", "-a", "-d", "-q"])?;
    git(mirror, &["prune", "--expire=now"])?;
    Ok(())
}

/// Compacts the mirror of every Git remote (`hb gc`).
pub fn compact_mirrors(ws: &Workspace) -> Result<usize> {
    let dir = ws.repo_dir.join("git");
    let mut done = 0;
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.join("HEAD").is_file() {
            compact_mirror(&path)?;
            done += 1;
        }
    }
    Ok(done)
}

/// Makes the mirror whole again (a push built on a commit older than the
/// tips it kept).
fn unshallow(mirror: &Path, url: &str) -> Result<()> {
    if mirror.join("shallow").is_file() {
        git(mirror, &["fetch", "--quiet", "--unshallow", "--", url])?;
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct Fetched {
    /// Branches of the remote, as Hebra commits.
    pub bookmarks: BTreeMap<String, ObjectId>,
    pub objects: usize,
    /// Whether the new objects are already compacted (read straight from
    /// Git's packs, with its deltas), so a clone needs no `hb gc`.
    pub compact: bool,
}

/// What an import of Git objects brought in.
struct MirrorImport {
    commits: usize,
    tags: BTreeMap<String, Tag>,
    compact: bool,
}

/// Imports Git history by reading its objects directly when that works
/// (much faster for whole histories), else through `git fast-export`.
/// Both give the same commits.
pub fn import_git_objects(
    tx: &mut Tx<'_>,
    git_dir: &Path,
    refs: &[(String, String)],
) -> hebra_core::Result<Option<gitimport::Imported>> {
    if !hebra_core::gitodb::readable(git_dir) || std::env::var_os("HB_GIT_FAST_EXPORT").is_some() {
        return Ok(None);
    }
    let db = tx.db();
    let known = |sha: &str| git_commit(db, sha);
    let store = tx.store;
    match store.batch(|| gitimport::import_objects(tx, git_dir, refs, &known)) {
        Ok(imported) => Ok(Some(imported)),
        // Something this reader does not handle: fall back (unless asked
        // not to, to find out why).
        Err(hebra_core::Error::InvalidArgument(_))
            if std::env::var_os("HB_GIT_OBJECTS_STRICT").is_none() =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn import_mirror(
    tx: &mut Tx<'_>,
    mirror: &Path,
    remote: &str,
    refs: &[MirrorRef],
) -> Coded<MirrorImport> {
    let io = |source: std::io::Error| hebra_core::Error::Io {
        path: "git fast-export".into(),
        source,
    };
    let db = tx.db();
    if refs.iter().all(|r| git_commit(db, &r.commit).is_some()) {
        return Ok(MirrorImport {
            commits: 0,
            tags: BTreeMap::new(),
            compact: false,
        });
    }
    let tips: Vec<ObjectId> = tx
        .view
        .remotes
        .get(remote)
        .into_iter()
        .flat_map(|r| r.values().copied())
        .chain(tx.view.tags.values().map(|t| t.target))
        .collect();
    // A first fetch reads the whole history: straight from the packs.
    if tips.is_empty() {
        let wanted: Vec<(String, String)> = refs
            .iter()
            .map(|r| (r.name.clone(), r.object.clone()))
            .collect();
        if let Some(imported) = import_git_objects(tx, mirror, &wanted)? {
            tx.git_map_insert(&imported.shas)?;
            return Ok(MirrorImport {
                commits: imported.stats.commits,
                tags: imported.tags,
                compact: true,
            });
        }
    }
    let db = tx.db();
    let mut excluded = BTreeSet::new();
    for id in tips {
        if let Some(sha) = git_sha(db, &id)? {
            excluded.insert(format!("^{sha}"));
        }
    }
    let excluded: Vec<String> = excluded.into_iter().take(MAX_EXCLUDES).collect();
    let mut child = git_command(mirror)
        .args([
            "fast-export",
            "--show-original-ids",
            "--signed-tags=strip",
            "--tag-of-filtered-object=drop",
            "--reencode=yes",
            "--all",
        ])
        .args(&excluded)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| hebra_core::Error::InvalidArgument("git has no stdout".to_owned()))?;
    let store = tx.store;
    let known = |sha: &str| git_commit(db, sha);
    let imported = store.batch(|| gitimport::import(tx, BufReader::new(stdout), &known))?;
    let out = child.wait_with_output().map_err(io)?;
    if !out.status.success() {
        return Err(hebra_core::Error::InvalidArgument(format!(
            "git fast-export failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    tx.git_map_insert(&imported.shas)?;
    Ok(MirrorImport {
        commits: imported.stats.commits,
        tags: imported.tags,
        compact: false,
    })
}

fn fetch_git(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    depth: Option<u32>,
    only_branch: Option<&str>,
) -> Result<Fetched> {
    let url = url_of(ws, name)?;
    let mirror = ensure_mirror(ws, name)?;
    let depth = depth.map(|d| format!("--depth={d}"));
    let single = only_branch.map(|b| format!("+refs/heads/{b}:refs/heads/{b}"));
    let mut args = vec!["fetch", "--quiet", "--force", "--update-head-ok"];
    if let Some(depth) = &depth {
        args.push(depth);
    }
    if single.is_none() {
        args.push("--prune");
    }
    args.push("--");
    args.push(&url);
    match &single {
        Some(refspec) => args.push(refspec),
        None => args.extend(["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"]),
    }
    git(&mirror, &args)?;
    let refs = mirror_refs(&mirror)?;
    let remote = name.to_owned();
    ws.run(actor, prov, "fetch", |tx| {
        let imported = import_mirror(tx, &mirror, &remote, &refs)?;
        let (objects, annotated, compact) = (imported.commits, imported.tags, imported.compact);
        let db = tx.db();
        let mut bookmarks = BTreeMap::new();
        for MirrorRef {
            name: reference,
            commit: sha,
            ..
        } in &refs
        {
            let Some(id) = git_commit(db, sha) else {
                continue;
            };
            if let Some(branch) = reference.strip_prefix("refs/heads/") {
                bookmarks.insert(branch.to_owned(), id);
            } else if let Some(tag) = reference.strip_prefix("refs/tags/") {
                let entry = annotated.get(tag).cloned().unwrap_or(Tag {
                    target: id,
                    message: None,
                    tagger: None,
                    timestamp: None,
                    signature: None,
                });
                tx.view.tags.entry(tag.to_owned()).or_insert(entry);
            }
        }
        tx.view.remotes.insert(remote.clone(), bookmarks.clone());
        Ok(Fetched {
            bookmarks,
            objects,
            compact,
        })
    })
    .map_err(Into::into)
}

/// Where a remote's default branch is.
fn git_default_branch(url: &str, cwd: &Path) -> Option<String> {
    let listing = git(cwd, &["ls-remote", "--symref", "--", url, "HEAD"]).ok()?;
    listing.lines().find_map(|line| {
        line.strip_prefix("ref: refs/heads/")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_owned)
    })
}

fn push_git(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    bookmarks: &[String],
    force: bool,
    tags: bool,
) -> Result<Output> {
    match push_git_once(ws, actor, prov, name, bookmarks, force, tags) {
        // A compacted mirror lacks a commit this push builds on.
        Err(e) if format!("{e:#}").contains("fast-import") => {
            let mirror = ensure_mirror(ws, name)?;
            if !mirror.join("shallow").is_file() {
                return Err(e);
            }
            unshallow(&mirror, &url_of(ws, name)?)?;
            push_git_once(ws, actor, prov, name, bookmarks, force, tags)
        }
        other => other,
    }
}

fn push_git_once(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    bookmarks: &[String],
    force: bool,
    tags: bool,
) -> Result<Output> {
    let url = url_of(ws, name)?;
    let mirror = ensure_mirror(ws, name)?;
    let marks = tempfile::NamedTempFile::new()?;
    let marks_path = marks.path().to_path_buf();
    let wanted = bookmarks.to_vec();
    let signer = crate::sign::configured(ws);
    let mut unsigned: Option<String> = None;
    let (refs, stats) = ws.run(actor, prov, "push", |tx| {
        let io = |source: std::io::Error| hebra_core::Error::Io {
            path: "git fast-import".into(),
            source,
        };
        let mut refs = Vec::new();
        for name in &wanted {
            let id = tx.resolve(name)?;
            refs.push((format!("refs/heads/{name}"), id));
        }
        if tags {
            for (name, tag) in &tx.view.tags {
                refs.push((format!("refs/tags/{name}"), tag.target));
            }
        }
        let mut child = git_command(&mirror)
            .args(["fast-import", "--quiet", "--force"])
            .arg(format!("--export-marks={}", marks_path.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io)?;
        let exported = {
            let stdin = child.stdin.take().ok_or_else(|| {
                hebra_core::Error::InvalidArgument("git fast-import has no stdin".to_owned())
            })?;
            let mut buffered = BufWriter::new(stdin);
            let db = tx.db();
            let known = |id: &ObjectId| git_sha(db, id).ok().flatten();
            let exported = gitexport::write_refs(tx, &refs, &known, &mut buffered)?;
            buffered.flush().map_err(io)?;
            exported
        };
        let out = child.wait_with_output().map_err(io)?;
        if !out.status.success() {
            return Err(hebra_core::Error::InvalidArgument(format!(
                "git fast-import failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let text = std::fs::read_to_string(&marks_path).map_err(io)?;
        let shas: BTreeMap<u64, String> = text
            .lines()
            .filter_map(|l| {
                let (mark, sha) = l.split_once(' ')?;
                Some((mark.trim_start_matches(':').parse().ok()?, sha.to_owned()))
            })
            .collect();
        let mut ordered = exported.marks.clone();
        ordered.sort_unstable_by_key(|(mark, _)| *mark);
        let mut pairs: Vec<(ObjectId, String)> = ordered
            .iter()
            .filter_map(|(mark, id)| shas.get(mark).map(|sha| (*id, sha.clone())))
            .collect();

        // Commits signed here are signed again over Git's bytes.
        let mut commits = Vec::with_capacity(pairs.len());
        for (id, sha) in &pairs {
            commits.push((sha.clone(), tx.commit(id)?.signature.is_some()));
        }
        if commits.iter().any(|(_, signed)| *signed) {
            match sign_for_git(&mirror, &signer, &commits) {
                Ok(rewrites) => {
                    for (_, sha) in &mut pairs {
                        if let Some(new) = rewrites.get(sha) {
                            sha.clone_from(new);
                        }
                    }
                    for (reference, _) in &refs {
                        let Ok(at) = git(&mirror, &["rev-parse", reference]) else {
                            continue;
                        };
                        if let Some(new) = rewrites.get(at.trim()) {
                            git(&mirror, &["update-ref", reference, new]).map_err(|e| {
                                hebra_core::Error::InvalidArgument(format!("{e:#}"))
                            })?;
                        }
                    }
                }
                Err(e) => unsigned = Some(format!("{e:#}")),
            }
        }
        tx.git_map_insert(&pairs)?;
        Ok((refs, exported.stats))
    })?;

    let mut args: Vec<String> = vec!["push".into(), "--quiet".into()];
    if force {
        args.push("--force".into());
    }
    args.push("--".into());
    args.push(url.clone());
    for (reference, _) in &refs {
        args.push(format!("{reference}:{reference}"));
    }
    let view: Vec<&str> = args.iter().map(String::as_str).collect();
    git(&mirror, &view).map_err(|e| {
        anyhow!(
            "{e:#}\nRun `hb pull {name}` and try again, or use --force to overwrite the remote."
        )
    })?;

    let remote = name.to_owned();
    let pushed = refs.clone();
    ws.run(actor, prov, "push", |tx| {
        let tracking = tx.view.remotes.entry(remote.clone()).or_default();
        for (reference, id) in &pushed {
            if let Some(branch) = reference.strip_prefix("refs/heads/") {
                tracking.insert(branch.to_owned(), *id);
            }
        }
        Ok(())
    })?;
    let mut text = format!("Pushed to {name} ({} new commit(s))\n", stats.commits);
    for (reference, id) in &refs {
        let _ = writeln!(text, "  {reference} -> {}", id.short());
    }
    if let Some(reason) = &unsigned {
        let _ = writeln!(
            text,
            "warning: signed commits reached Git unsigned: {reason}"
        );
    }
    Ok(Output::new(
        json!({ "remote": name, "commits": stats.commits, "unsigned": unsigned, "pushed": refs.iter().map(|(r, id)| json!({ "ref": r, "commit": id.hex() })).collect::<Vec<_>>() }),
        text,
    ))
}

// ------------------------------------------------- another repository here

fn copy_history(
    from: &Workspace,
    to: &Workspace,
    wants: &[ObjectId],
    haves: &HashSet<ObjectId>,
) -> Result<usize> {
    let ids = transport::closure(&from.store, wants, haves)?;
    let store = to.store.clone();
    let mut new = 0;
    let source = from.store.clone();
    store.batch(|| {
        for id in &ids {
            if !store.has(id) {
                store.write_raw(&source.read_raw(id)?)?;
                new += 1;
            }
        }
        Ok(())
    })?;
    Ok(new)
}

fn fetch_local(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
) -> Result<Fetched> {
    let url = url_of(ws, name)?;
    let other = Workspace::open(Path::new(&url))
        .with_context(|| format!("opening the repository at {url}"))?;
    let theirs = other.view()?;
    let tracking = ws.view()?.remotes.get(name).cloned().unwrap_or_default();
    let haves: HashSet<ObjectId> = tracking.values().copied().collect();
    let mut wants: Vec<ObjectId> = theirs.bookmarks.values().copied().collect();
    wants.extend(theirs.tags.values().map(|t| t.target));
    let objects = copy_history(&other, ws, &wants, &haves)?;
    let remote = name.to_owned();
    let bookmarks = theirs.bookmarks.clone();
    let tags = theirs.tags.clone();
    ws.run(actor, prov, "fetch", |tx| {
        tx.view.remotes.insert(remote, bookmarks.clone());
        for (name, tag) in tags {
            tx.view.tags.entry(name).or_insert(tag);
        }
        Ok(Fetched {
            bookmarks,
            objects,
            compact: false,
        })
    })
    .map_err(Into::into)
}

fn push_local(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    bookmarks: &[String],
    force: bool,
) -> Result<Output> {
    let url = url_of(ws, name)?;
    let mut other = Workspace::open(Path::new(&url))
        .with_context(|| format!("opening the repository at {url}"))?;
    let view = ws.view()?;
    let mut sent = Vec::new();
    let mut text = String::new();
    for bookmark in bookmarks {
        let local = *view
            .bookmarks
            .get(bookmark)
            .ok_or_else(|| anyhow!("no bookmark `{bookmark}`"))?;
        let their_view = other.view()?;
        let haves: HashSet<ObjectId> = their_view
            .bookmarks
            .values()
            .copied()
            .filter(|id| ws.store.has(id))
            .collect();
        copy_history(ws, &other, &[local], &haves)?;
        let target = bookmark.clone();
        other.run(actor, prov, "receive", |tx| {
            if let Some(current) = tx.view.bookmarks.get(&target).copied()
                && !force
                && !tx.is_ancestor(current, local)?
            {
                return Err(hebra_core::Error::InvalidArgument(format!(
                    "`{target}` moved on the remote: pull first, or use --force"
                )));
            }
            tx.view.bookmarks.insert(target.clone(), local);
            Ok(())
        })?;
        let _ = writeln!(text, "{bookmark} -> {} on {name}", local.short());
        sent.push(json!({ "bookmark": bookmark, "commit": local.hex() }));
    }
    let remote = name.to_owned();
    let names = bookmarks.to_vec();
    ws.run(actor, prov, "push", |tx| {
        for bookmark in &names {
            if let Some(id) = tx.view.bookmarks.get(bookmark).copied() {
                tx.view
                    .remotes
                    .entry(remote.clone())
                    .or_default()
                    .insert(bookmark.clone(), id);
            }
        }
        Ok(())
    })?;
    Ok(Output::new(json!({ "remote": name, "pushed": sent }), text))
}

// ------------------------------------------------------------ any kind

/// Downloads what a remote has, without touching local bookmarks.
pub fn fetch(ws: &mut Workspace, actor: &Actor, prov: &Provenance, name: &str) -> Result<Fetched> {
    match kind(ws, name)? {
        Kind::Git => fetch_git(ws, actor, prov, name, None, None),
        Kind::Local => fetch_local(ws, actor, prov, name),
        Kind::Hebra => crate::remote::fetch(ws, actor, prov, name),
    }
}

pub fn fetch_cmd(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    all: bool,
) -> Result<Output> {
    let names: Vec<String> = if all {
        ws.config_list()?
            .keys()
            .filter_map(|k| {
                k.strip_prefix("remote.")
                    .and_then(|k| k.strip_suffix(".url"))
                    .map(str::to_owned)
            })
            .collect()
    } else {
        vec![name.to_owned()]
    };
    let mut text = String::new();
    let mut rows = Vec::new();
    for remote in names {
        let fetched = fetch(ws, actor, prov, &remote)?;
        let _ = writeln!(
            text,
            "Fetched {remote}: {} bookmark(s), {} new.",
            fetched.bookmarks.len(),
            fetched.objects
        );
        rows.push(json!({ "remote": remote, "bookmarks": fetched.bookmarks.keys().collect::<Vec<_>>(), "objects": fetched.objects }));
    }
    Ok(Output::new(json!({ "fetched": rows }), text))
}

#[derive(Debug, Clone, Default)]
pub struct PullMode {
    pub rebase: bool,
    pub merge: bool,
    pub ff_only: bool,
    pub signer: Option<Signer>,
}

fn integrate(
    tx: &mut Tx<'_>,
    remote: &str,
    server: &BTreeMap<String, ObjectId>,
    tracked: &BTreeMap<String, ObjectId>,
    mode: &PullMode,
) -> Coded<(Vec<String>, Vec<String>)> {
    let mut updated = Vec::new();
    let mut diverged = Vec::new();
    for (name, new) in server {
        match tx.view.bookmarks.get(name).copied() {
            None => {
                tx.view.bookmarks.insert(name.clone(), *new);
                updated.push(name.clone());
            }
            Some(local) if local == *new => {}
            Some(local) => {
                // A bookmark nobody moved here follows the remote anywhere.
                if tracked.get(name) == Some(&local) || tx.is_ancestor(local, *new)? {
                    tx.view.bookmarks.insert(name.clone(), *new);
                    updated.push(name.clone());
                } else if !tx.is_ancestor(*new, local)? {
                    diverged.push(name.clone());
                }
            }
        }
    }
    if let Some(current) = tx.current_bookmark() {
        if updated.contains(&current) {
            let target = tx.view.bookmarks[&current];
            porcelain::follow(tx, target, &current)?;
        } else if diverged.contains(&current) {
            let theirs = server[&current];
            if mode.ff_only {
                return Err(hebra_core::Error::InvalidArgument(format!(
                    "`{current}` has diverged from {remote}: not possible to fast-forward"
                )));
            }
            if mode.rebase {
                let wc = tx.wc_id()?;
                let commits = tx.range(Some(theirs), wc)?;
                let moved = tx.rebase(&commits, theirs)?;
                if let Some(signer) = &mode.signer {
                    let after = commits.iter().filter_map(|id| moved.get(id).copied());
                    porcelain::sign_rebased(tx, &commits, after.collect(), signer)?;
                }
                let at = tx.wc_id()?;
                let tip = tx.commit(&at)?.parents.first().copied().unwrap_or(theirs);
                tx.view.bookmarks.insert(current.clone(), tip);
                diverged.retain(|n| *n != current);
            } else if mode.merge {
                porcelain::merge_into_wc(tx, theirs)?;
                diverged.retain(|n| *n != current);
            }
        }
    }
    Ok((updated, diverged))
}

pub fn pull(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    mode: &PullMode,
) -> Result<Output> {
    let tracked_before = ws.view()?.remotes.get(name).cloned().unwrap_or_default();
    let fetched = fetch(ws, actor, prov, name)?;
    let server = fetched.bookmarks.clone();
    let remote = name.to_owned();
    let (updated, diverged) = ws.run(actor, prov, "pull", |tx| {
        let (updated, diverged) = integrate(tx, &remote, &server, &tracked_before, mode)?;
        tx.view.remotes.insert(remote.clone(), server.clone());
        Ok((updated, diverged))
    })?;
    let mut text = format!(
        "Pulled from {name}: {} bookmark(s), {} new object(s).\n",
        fetched.bookmarks.len(),
        fetched.objects
    );
    for bookmark in &updated {
        let _ = writeln!(
            text,
            "  {bookmark} -> {}",
            fetched.bookmarks[bookmark].short()
        );
    }
    for bookmark in &diverged {
        let _ = writeln!(
            text,
            "  {bookmark} has diverged: merge it with `hb merge {bookmark}@{name}`, or use `hb pull --rebase`"
        );
    }
    Ok(Output::new(
        json!({ "remote": name, "bookmarks": fetched.bookmarks.keys().collect::<Vec<_>>(), "updated": updated, "diverged": diverged, "objects": fetched.objects }),
        text,
    ))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PushOptions {
    pub force: bool,
    pub tags: bool,
    pub all: bool,
}

/// Pushes to a remote that is not a Hebra server.
pub fn push(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    name: &str,
    branches: &[String],
    options: PushOptions,
) -> Result<Output> {
    let PushOptions { force, tags, all } = options;
    let view = ws.view()?;
    let bookmarks: Vec<String> = if all {
        view.bookmarks.keys().cloned().collect()
    } else if !branches.is_empty() {
        branches.to_vec()
    } else if let Some(current) = view
        .workspaces
        .get(&ws.name)
        .and_then(|w| w.bookmark.clone())
    {
        vec![current]
    } else {
        let tracked = view.remotes.get(name).cloned().unwrap_or_default();
        view.bookmarks
            .iter()
            .filter(|(n, id)| tracked.get(*n) != Some(id))
            .map(|(n, _)| n.clone())
            .collect()
    };
    if bookmarks.is_empty() && !tags {
        return Ok(Output::new(
            json!({ "pushed": [] }),
            "Nothing to push.\n".to_owned(),
        ));
    }
    match kind(ws, name)? {
        Kind::Git => push_git(ws, actor, prov, name, &bookmarks, force, tags),
        Kind::Local => push_local(ws, actor, prov, name, &bookmarks, force),
        Kind::Hebra => bail!("not a Git remote"),
    }
}

// ----------------------------------------------------------------- clone

fn directory_name(url: &str) -> String {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    trimmed
        .rsplit(['/', '\\', ':'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("repository")
        .to_owned()
}

fn default_branch(
    wanted: Option<&str>,
    found: Option<String>,
    bookmarks: &BTreeMap<String, ObjectId>,
) -> Option<String> {
    wanted
        .map(str::to_owned)
        .or(found)
        .filter(|b| bookmarks.contains_key(b))
        .or_else(|| {
            ["main", "master", "trunk"]
                .iter()
                .find(|b| bookmarks.contains_key(**b))
                .map(|b| (*b).to_owned())
        })
        .or_else(|| bookmarks.keys().next().cloned())
}

pub fn clone(args: &CloneArgs, cwd: &Path) -> Result<Output> {
    let found = if !args.git && is_local_hebra(&cwd.join(&args.url).display().to_string()) {
        Kind::Local
    } else {
        kind_of_url(&args.url, args.git)
    };
    if args.filter.is_some() && found != Kind::Hebra {
        bail!("--filter needs a Hebra server; for Git remotes use --depth");
    }
    let dest = cwd.join(
        args.directory
            .clone()
            .unwrap_or_else(|| PathBuf::from(directory_name(&args.url))),
    );
    if dest.exists() && std::fs::read_dir(&dest)?.next().is_some() {
        bail!("{} already exists and is not empty", dest.display());
    }
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let (actor, prov) = crate::commands::fresh_identity();
    let mut ws = Workspace::init(&dest, &actor, &prov)?;
    let name = args.origin.clone();
    let url = if found == Kind::Local {
        std::path::absolute(cwd.join(&args.url))?
            .display()
            .to_string()
    } else {
        args.url.clone()
    };
    if let Some(text) = kind_name(found) {
        ws.config_set(&format!("remote.{name}.url"), &url)?;
        ws.config_set(&format!("remote.{name}.kind"), text)?;
    } else {
        crate::remote::add(&ws, &name, &url, args.token.clone())?;
    }
    let single = args
        .single_branch
        .then_some(args.branch.as_deref())
        .flatten();
    // Which branch the remote calls its default: asked while fetching.
    let probe = (found == Kind::Git).then(|| {
        let (url, dest) = (url.clone(), dest.clone());
        std::thread::spawn(move || git_default_branch(&url, &dest))
    });
    let fetched = match found {
        Kind::Git => fetch_git(&mut ws, &actor, &prov, &name, args.depth, single)?,
        Kind::Local => fetch_local(&mut ws, &actor, &prov, &name)?,
        Kind::Hebra => {
            if args.filter.as_deref().is_some_and(|f| f != "blob:none") {
                bail!("only --filter=blob:none is supported");
            }
            if args.filter.is_some() {
                ws.config_set(&format!("remote.{name}.partial"), "true")?;
                crate::remote::enable_partial(&ws)?;
            }
            crate::remote::fetch_filtered(&mut ws, &actor, &prov, &name, args.filter.is_some())?
        }
    };
    let found_default = probe.and_then(|p| p.join().ok().flatten());
    let branch = default_branch(args.branch.as_deref(), found_default, &fetched.bookmarks);
    let summary = ws.run(&actor, &prov, "clone", |tx| {
        let Some(branch) = &branch else {
            return Ok(None);
        };
        let target = fetched.bookmarks[branch];
        tx.view.bookmarks.insert(branch.clone(), target);
        if args.no_checkout {
            return Ok(Some(target));
        }
        porcelain::follow(tx, target, branch)?;
        Ok(Some(target))
    })?;
    if fetched.compact && found == Kind::Git {
        compact_mirror(&mirror_path(&ws, &name))?;
    }
    // A clone starts compact: everything is in one delta-compressed pack.
    if !fetched.compact && (found != Kind::Hebra || args.filter.is_none()) {
        let _ = ws.gc(std::time::SystemTime::now(), None);
    }
    if args.recurse_submodules {
        crate::extras::submodule_update(&ws, true)?;
    }
    let mut text = format!("Cloned {} into {}\n", args.url, dest.display());
    if let (Some(branch), Some(tip)) = (&branch, summary) {
        let commit = ws.look(&actor, &prov, |tx| {
            let c = tx.commit(&tip)?;
            Ok(short_line(&tip, &c))
        })?;
        let _ = writeln!(text, "On branch {branch} at {commit}");
    } else {
        let _ = writeln!(text, "The repository is empty.");
    }
    Ok(Output::new(
        json!({ "path": dest.display().to_string(), "remote": name, "kind": kind_name(found).unwrap_or("hebra"), "branch": branch, "bookmarks": fetched.bookmarks.keys().collect::<Vec<_>>() }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_urls_are_told_apart_from_hebra_servers() {
        for url in [
            "https://github.com/owner/repo",
            "https://gitlab.com/a/b.git",
            "git@github.com:owner/repo.git",
            "git@example.com:repo",
            "ssh://git@host/repo",
            "https://example.com/x/y.git",
        ] {
            assert!(is_git_url(url), "{url}");
        }
        for url in [
            "https://hebra.example.com/ana/acme",
            "http://localhost:4600/a/b",
        ] {
            assert!(!is_git_url(url), "{url}");
        }
    }

    #[test]
    fn clone_directories_come_from_the_url() {
        assert_eq!(directory_name("https://github.com/a/b.git"), "b");
        assert_eq!(directory_name("git@host:a/repo.git"), "repo");
        assert_eq!(directory_name("../local/thing/"), "thing");
    }
}
