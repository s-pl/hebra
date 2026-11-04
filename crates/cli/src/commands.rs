//! Command implementations. Each returns an [`Output`] with both a JSON value
//! (for agents) and text (for people), so the CLI and the MCP server share
//! exactly the same behaviour.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use hebra_core::claims::{self, Claim};
use hebra_core::diff::FileStatus;
use hebra_core::identity;
use hebra_core::model::{Actor, ActorKind, Commit, Conflict, Provenance};
use hebra_core::repo::DEFAULT_WORKSPACE;
use hebra_core::{Error, ObjectId, Tx, Workspace};
use serde_json::{Value, json};

use crate::cli::{
    BookmarkCommand, BundleCommand, ChecksCommand, Command, ConfigArgs, ConfigCommand, GitCommand,
    HookCommand, NotesCommand, OpCommand, RemoteCommand, ReviewCommand, StartArgs, StashCommand,
    TaskCommand, WorkspaceCommand, WorktreeCommand,
};
use crate::fmt::{ago, parse_ttl, slug};
use crate::{bridge, extras, hooks, inspect, porcelain, sign, transfer};

pub struct Output {
    pub json: Value,
    pub text: String,
    /// Holds a diff, to color at a terminal.
    pub diff: bool,
    /// Bytes to write as they are instead of `text` (file contents).
    pub raw: Option<Vec<u8>>,
    /// Exit code, for commands that answer with it (`merge-base --is-ancestor`).
    pub exit: u8,
}

impl Output {
    pub(crate) fn new(json: Value, text: impl Into<String>) -> Self {
        Self {
            json,
            text: text.into(),
            diff: false,
            raw: None,
            exit: 0,
        }
    }

    pub(crate) fn with_diff(mut self) -> Self {
        self.diff = true;
        self
    }

    pub(crate) fn binary(json: Value, raw: Vec<u8>) -> Self {
        Self {
            json,
            text: String::new(),
            diff: false,
            raw: Some(raw),
            exit: 0,
        }
    }

    pub(crate) fn with_exit(json: Value, text: impl Into<String>, exit: u8) -> Self {
        Self {
            json,
            text: text.into(),
            diff: false,
            raw: None,
            exit,
        }
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// A setting from the `[user]` section of Git's config files. Read directly:
/// starting `git` on every command would cost more than the command itself.
fn git_user_setting(key: &str) -> Option<String> {
    let home = env("HOME").or_else(|| env("USERPROFILE"))?;
    let home = std::path::PathBuf::from(home);
    let xdg = env("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), Into::into);
    [home.join(".gitconfig"), xdg.join("git").join("config")]
        .iter()
        .find_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let mut in_user = false;
            let mut found = None;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_user = line.eq_ignore_ascii_case("[user]");
                } else if in_user
                    && let Some((name, value)) = line.split_once('=')
                    && name.trim().eq_ignore_ascii_case(key)
                {
                    found = Some(value.trim().trim_matches('"').to_owned());
                }
            }
            found.filter(|v| !v.is_empty())
        })
}

/// A `user.*` setting from Hebra's config, falling back to Git's.
fn configured(ws: Option<&Workspace>, key: &str) -> Option<String> {
    ws.and_then(|w| w.config_get(&format!("user.{key}")).ok().flatten())
        .or_else(|| crate::settings::get(&format!("user.{key}")))
        .or_else(|| git_user_setting(key))
}

fn configured_user(ws: Option<&Workspace>) -> Option<String> {
    configured(ws, "name")
}

pub(crate) fn identity_for(ws: &Workspace) -> Result<(Actor, Provenance)> {
    let info = ws.info()?;
    let mut actor = identity::resolve_actor(env, info.as_ref(), configured_user(Some(ws)));
    if actor.kind == ActorKind::Human {
        actor.email = configured(Some(ws), "email");
    }
    let provenance = identity::provenance(env, info.as_ref());
    Ok((actor, provenance))
}

pub fn execute(command: Command, cwd: &Path) -> Result<Output> {
    if let Command::Init { path, .. } = &command {
        return init(&cwd.join(path.as_deref().unwrap_or(Path::new("."))));
    }
    if let Command::Clone(args) = &command {
        return bridge::clone(args, cwd);
    }
    if let Command::Version = &command {
        let version = env!("CARGO_PKG_VERSION");
        return Ok(Output::new(
            json!({ "version": version }),
            format!("hb {version}\n"),
        ));
    }
    if let Command::Config(args) = &command
        && args.global
    {
        return extras::config_global(
            args.key.as_deref(),
            args.value.as_deref(),
            args.unset,
            args.list,
        );
    }
    let mut ws = Workspace::open(cwd)?;
    crate::remote::enable_partial(&ws)?;
    let (actor, prov) = identity_for(&ws)?;
    let name = command_name(&command);
    let name = name.as_str();
    match command {
        Command::Init { .. } | Command::Mcp | Command::Clone(_) | Command::Version => {
            unreachable!("handled by the caller")
        }
        Command::Status(args) => {
            if args.short || args.porcelain {
                return ws
                    .read(&actor, &prov, |tx| inspect::status_short(tx, &args))
                    .map_err(Into::into);
            }
            ws.snapshot_others(&prov, false)?;
            let mut out = ws.read(&actor, &prov, status)?;
            add_remote_warnings(&mut ws, &actor, &prov, &mut out);
            Ok(out)
        }
        Command::Diff(args) => ws
            .read(&actor, &prov, |tx| inspect::diff_cmd(tx, &args))
            .map_err(Into::into),
        Command::Describe {
            revision,
            message: Some(message),
            ..
        } => ws
            .run(&actor, &prov, name, |tx| describe(tx, &revision, message))
            .map_err(Into::into),
        Command::Describe {
            revision,
            message: None,
            always,
        } => {
            let revision = if revision == "@" {
                "@-".to_owned()
            } else {
                revision
            };
            ws.look(&actor, &prov, |tx| {
                porcelain::describe_tag(tx, &revision, always)
            })
            .map_err(Into::into)
        }
        Command::Commit(args) => porcelain::commit(&mut ws, &actor, &prov, &args, cwd),
        Command::New { revisions, message } => ws
            .run(&actor, &prov, name, |tx| new(tx, &revisions, message))
            .map_err(Into::into),
        Command::Log(args) => ws
            .read(&actor, &prov, |tx| inspect::log(tx, &args))
            .map_err(Into::into),
        Command::Show(args) => ws
            .read(&actor, &prov, |tx| inspect::show(tx, &args))
            .map_err(Into::into),
        Command::Merge(args) => {
            if args.abort {
                bail!("nothing to abort: a merge never stops half-way. `hb undo` takes it back");
            }
            if args.sign.gpg_sign.is_some() {
                bail!("`hb commit` finishes a merge: sign it there, with `hb commit -S`");
            }
            if args.resume {
                return Ok(Output::new(
                    json!({ "continued": false }),
                    "Nothing to continue: conflicts are recorded in the change. Edit the files, then `hb commit`.\n"
                        .to_owned(),
                ));
            }
            ws.run(&actor, &prov, name, |tx| porcelain::merge(tx, &args))
                .map_err(Into::into)
        }
        Command::Resolve {
            paths,
            ours,
            theirs,
        } => {
            let paths = porcelain::tree_paths(&ws, cwd, &paths)?;
            ws.run(&actor, &prov, name, |tx| {
                porcelain::resolve(tx, &paths, ours, theirs)
            })
            .map_err(Into::into)
        }
        Command::Add { .. } => Ok(porcelain::add()),
        Command::Rm {
            paths,
            cached,
            recursive,
            force: _,
        } => porcelain::rm(&mut ws, &actor, &prov, cwd, &paths, cached, recursive),
        Command::Mv { paths, force } => {
            let out = porcelain::mv(&ws, cwd, &paths, force)?;
            ws.snapshot(&actor, &prov)?;
            Ok(out)
        }
        Command::Restore { staged: true, .. } => Ok(porcelain::add()),
        Command::Restore { paths, source, .. } => {
            let paths = porcelain::tree_paths(&ws, cwd, &paths)?;
            ws.run(&actor, &prov, name, |tx| {
                porcelain::restore(tx, &paths, &source)
            })
            .map_err(Into::into)
        }
        Command::Reset {
            revision,
            hard,
            soft: _,
            mixed: _,
        } => {
            // `git reset <file>` unstages it: there is nothing to unstage.
            if cwd.join(&revision).exists()
                && ws.look(&actor, &prov, |tx| tx.resolve(&revision)).is_err()
            {
                return Ok(porcelain::add());
            }
            ws.run(&actor, &prov, name, |tx| {
                porcelain::reset(tx, &revision, hard)
            })
            .map_err(Into::into)
        }
        Command::Clean {
            dry_run,
            force,
            d: _,
            x,
            only_ignored,
        } => porcelain::clean(&mut ws, &actor, &prov, dry_run, force, x, only_ignored),
        Command::Checkout {
            target,
            create,
            paths,
        } => {
            // `git checkout .` and `git checkout <file>` restore files.
            let names_files = target.as_deref().is_some_and(|t| {
                t == "."
                    || (cwd.join(t).exists() && ws.look(&actor, &prov, |tx| tx.resolve(t)).is_err())
            });
            if names_files {
                let path = target.clone().unwrap_or_default();
                let paths = porcelain::tree_paths(&ws, cwd, &[path])?;
                return ws
                    .run(&actor, &prov, name, |tx| {
                        porcelain::restore(tx, &paths, "@-")
                    })
                    .map_err(Into::into);
            }
            if !paths.is_empty() {
                let paths = porcelain::tree_paths(&ws, cwd, &paths)?;
                let source = target.unwrap_or_else(|| "@-".to_owned());
                return ws
                    .run(&actor, &prov, name, |tx| {
                        porcelain::restore(tx, &paths, &source)
                    })
                    .map_err(Into::into);
            }
            let out = ws.run(&actor, &prov, name, |tx| {
                porcelain::switch(tx, target.as_deref(), create.as_deref(), false)
            })?;
            hooks::run_optional(&ws, "post-checkout", &[]);
            Ok(out)
        }
        Command::Switch {
            target,
            create,
            detach,
        } => {
            let out = ws.run(&actor, &prov, name, |tx| {
                porcelain::switch(tx, target.as_deref(), create.as_deref(), detach)
            })?;
            hooks::run_optional(&ws, "post-checkout", &[]);
            Ok(out)
        }
        Command::Branch(args) => {
            let lists = !args.delete && !args.rename && args.name.is_none();
            if lists {
                ws.look(&actor, &prov, |tx| porcelain::branch(tx, &args))
                    .map_err(Into::into)
            } else {
                ws.run(&actor, &prov, name, |tx| porcelain::branch(tx, &args))
                    .map_err(Into::into)
            }
        }
        Command::Tag(args) => {
            let signer = sign::signer_for_tags(&ws, args.sign);
            if args.verify {
                let target = args.name.clone().unwrap_or_default();
                return extras::verify(&mut ws, &actor, &prov, &target);
            }
            ws.run(&actor, &prov, name, |tx| {
                porcelain::tag(tx, &args, signer.as_ref())
            })
            .map_err(Into::into)
        }
        Command::Stash { command } => {
            let cmd = match command {
                None => StashCommand::Push {
                    message: None,
                    include_untracked: true,
                    paths: Vec::new(),
                },
                Some(StashCommand::Push {
                    message,
                    include_untracked,
                    paths,
                }) => StashCommand::Push {
                    message,
                    include_untracked,
                    paths: porcelain::tree_paths(&ws, cwd, &paths)?,
                },
                Some(other) => other,
            };
            ws.run(&actor, &prov, name, |tx| porcelain::stash(tx, &cmd))
                .map_err(Into::into)
        }
        Command::Rebase(args) => {
            let plan = porcelain::rebase_plan(&mut ws, &actor, &prov, &args)?;
            hooks::run_required(&ws, "pre-rebase", &[])?;
            let signer = sign::wanted(&ws, &args.sign);
            ws.run(&actor, &prov, name, |tx| {
                porcelain::rebase(tx, &args, &plan, signer.as_ref())
            })
            .map_err(Into::into)
        }
        Command::CherryPick {
            revisions,
            no_commit,
            record_origin,
            mainline,
            sign,
        } => {
            let signer = sign::wanted(&ws, &sign);
            ws.run(&actor, &prov, name, |tx| {
                porcelain::cherry_pick(
                    tx,
                    &revisions,
                    no_commit,
                    record_origin,
                    mainline,
                    signer.as_ref(),
                )
            })
            .map_err(Into::into)
        }
        Command::Revert {
            revisions,
            no_commit,
            mainline,
            sign,
        } => {
            let signer = sign::wanted(&ws, &sign);
            ws.run(&actor, &prov, name, |tx| {
                porcelain::revert(tx, &revisions, no_commit, mainline, signer.as_ref())
            })
            .map_err(Into::into)
        }
        Command::Squash {
            from,
            into,
            message,
            paths,
        } => {
            let paths = porcelain::tree_paths(&ws, cwd, &paths)?;
            ws.run(&actor, &prov, name, |tx| {
                porcelain::squash(tx, &from, into.as_deref(), message, &paths)
            })
            .map_err(Into::into)
        }
        Command::Split {
            revision,
            message,
            paths,
        } => {
            let paths = porcelain::tree_paths(&ws, cwd, &paths)?;
            ws.run(&actor, &prov, name, |tx| {
                porcelain::split(tx, &revision, message, &paths)
            })
            .map_err(Into::into)
        }
        Command::Abandon { revisions } => ws
            .run(&actor, &prov, name, |tx| porcelain::abandon(tx, &revisions))
            .map_err(Into::into),
        Command::Edit { revision } => ws
            .run(&actor, &prov, name, |tx| porcelain::edit(tx, &revision))
            .map_err(Into::into),
        Command::Filter {
            remove,
            keep,
            revision,
        } => ws
            .run(&actor, &prov, name, |tx| {
                porcelain::filter(tx, &remove, &keep, &revision)
            })
            .map_err(Into::into),
        Command::Fetch { remote, all, .. } => {
            bridge::fetch_cmd(&mut ws, &actor, &prov, &remote, all)
        }
        Command::Blame {
            path,
            revision,
            lines,
        } => {
            let path = porcelain::tree_path(&ws, cwd, &path)?;
            ws.read(&actor, &prov, |tx| {
                inspect::blame_cmd(tx, &path, &revision, lines.as_deref())
            })
            .map_err(Into::into)
        }
        Command::Grep(mut args) => {
            args.paths = porcelain::tree_paths(&ws, cwd, &args.paths)?;
            ws.read(&actor, &prov, |tx| inspect::grep(tx, &args))
                .map_err(Into::into)
        }
        Command::Shortlog {
            summary,
            email,
            revision,
        } => ws
            .look(&actor, &prov, |tx| {
                inspect::shortlog(tx, summary, email, &revision)
            })
            .map_err(Into::into),
        Command::LsFiles {
            modified,
            deleted,
            ignored,
            paths,
        } => {
            if ignored {
                let files = hebra_core::wc::ignored_files(&ws.root)?;
                let text: String = files.iter().map(|f| format!("{f}\n")).collect();
                return Ok(Output::new(json!({ "files": files }), text));
            }
            let paths = porcelain::tree_paths(&ws, cwd, &paths)?;
            ws.read(&actor, &prov, |tx| {
                inspect::ls_files(tx, modified, deleted, &paths)
            })
            .map_err(Into::into)
        }
        Command::LsTree {
            revision,
            recursive,
            name_only,
            path,
        } => ws
            .read(&actor, &prov, |tx| {
                inspect::ls_tree(tx, &revision, recursive, name_only, path.as_deref())
            })
            .map_err(Into::into),
        Command::Cat { spec, kind, .. } => ws
            .read(&actor, &prov, |tx| inspect::cat(tx, &spec, kind))
            .map_err(Into::into),
        Command::RevParse {
            revisions,
            show_toplevel,
            git_dir,
            is_inside_work_tree,
            verify: _,
            abbrev_ref,
            short,
        } => {
            if show_toplevel {
                let root = ws.root.display().to_string();
                return Ok(Output::new(
                    json!({ "toplevel": root }),
                    format!("{root}\n"),
                ));
            }
            if git_dir {
                let dir = ws.repo_dir.display().to_string();
                return Ok(Output::new(json!({ "dir": dir }), format!("{dir}\n")));
            }
            if is_inside_work_tree {
                return Ok(Output::new(json!({ "inside": true }), "true\n".to_owned()));
            }
            ws.look(&actor, &prov, |tx| {
                inspect::rev_parse(tx, &revisions, abbrev_ref, short)
            })
            .map_err(Into::into)
        }
        Command::RevList {
            revisions,
            count,
            limit,
        } => ws
            .look(&actor, &prov, |tx| {
                inspect::rev_list(tx, &revisions, count, limit)
            })
            .map_err(Into::into),
        Command::MergeBase {
            a,
            b,
            is_ancestor,
            all,
        } => ws
            .look(&actor, &prov, |tx| {
                inspect::merge_base(tx, &a, &b, is_ancestor, all)
            })
            .map_err(Into::into),
        Command::ForEachRef { pattern } => ws
            .look(&actor, &prov, |tx| {
                inspect::for_each_ref(tx, pattern.as_deref())
            })
            .map_err(Into::into),
        Command::NameRev { revision } => ws
            .look(&actor, &prov, |tx| inspect::name_rev(tx, &revision))
            .map_err(Into::into),
        Command::Bisect(cmd) => extras::bisect(&mut ws, &actor, &prov, &cmd),
        Command::FormatPatch {
            revision,
            output_directory,
            stdout,
        } => {
            let dir = cwd.join(output_directory);
            std::fs::create_dir_all(&dir)?;
            ws.read(&actor, &prov, |tx| {
                transfer::format_patch(tx, &revision, &dir, stdout)
            })
            .map_err(Into::into)
        }
        Command::Apply {
            files,
            check,
            reverse,
        } => {
            let texts = transfer::read_inputs(cwd, &files)?;
            ws.run(&actor, &prov, name, |tx| {
                transfer::apply(tx, &texts, check, reverse)
            })
            .map_err(Into::into)
        }
        Command::Am { files, sign } => {
            let mails: Vec<transfer::Mail> = transfer::read_inputs(cwd, &files)?
                .iter()
                .flat_map(|text| transfer::parse_mails(text))
                .collect();
            transfer::require_mails(&mails)?;
            let signer = sign::wanted(&ws, &sign);
            ws.run(&actor, &prov, name, |tx| {
                transfer::am(tx, &mails, signer.as_ref())
            })
            .map_err(Into::into)
        }
        Command::Archive {
            revision,
            format,
            output,
            prefix,
        } => {
            let output = output.map(|o| cwd.join(o));
            ws.read(&actor, &prov, |tx| {
                transfer::archive_cmd(
                    tx,
                    &revision,
                    format.as_deref(),
                    output.as_deref(),
                    prefix.as_deref(),
                )
            })
            .map_err(Into::into)
        }
        Command::Bundle(BundleCommand::Create { file, bookmarks }) => {
            transfer::bundle_create(&ws, &cwd.join(file), &bookmarks)
        }
        Command::Bundle(BundleCommand::Unbundle { file }) => {
            transfer::bundle_unbundle(&mut ws, &actor, &prov, &cwd.join(file))
        }
        Command::Bundle(BundleCommand::ListHeads { file } | BundleCommand::Verify { file }) => {
            transfer::bundle_heads(&cwd.join(file))
        }
        Command::Notes { command } => match command.unwrap_or(NotesCommand::List) {
            NotesCommand::Add { revision, message } => ws
                .look(&actor, &prov, |tx| {
                    porcelain::notes_add(tx, &revision, &message)
                })
                .map_err(Into::into),
            NotesCommand::Show { revision } => ws
                .look(&actor, &prov, |tx| porcelain::notes_show(tx, &revision))
                .map_err(Into::into),
            NotesCommand::List => ws
                .look(&actor, &prov, porcelain::notes_list)
                .map_err(Into::into),
            NotesCommand::Remove { revision } => ws
                .look(&actor, &prov, |tx| porcelain::notes_remove(tx, &revision))
                .map_err(Into::into),
        },
        Command::Fsck => extras::fsck(&ws),
        Command::Sparse(cmd) => extras::sparse(&mut ws, &actor, &prov, &cmd),
        Command::Submodule { command } => extras::submodule(
            &mut ws,
            &actor,
            &prov,
            &command.unwrap_or(crate::cli::SubmoduleCommand::Status),
        ),
        Command::Reflog { limit } => op_log(&ws, limit),
        Command::Worktree(WorktreeCommand::List) => ws
            .look(&actor, &prov, |tx| Ok(extras::worktree_list(tx)))
            .map_err(Into::into),
        Command::Worktree(WorktreeCommand::Add { path, revision }) => {
            let path = cwd.join(path);
            ws.run(&actor, &prov, name, |tx| {
                extras::worktree_add(tx, &path, &revision)
            })
            .map_err(Into::into)
        }
        Command::Worktree(WorktreeCommand::Remove { name: target, keep }) => ws
            .run(&actor, &prov, name, |tx| {
                extras::worktree_remove(tx, &target, keep)
            })
            .map_err(Into::into),
        Command::Worktree(WorktreeCommand::Prune) => ws
            .run(&actor, &prov, name, extras::worktree_prune)
            .map_err(Into::into),
        Command::Stats => extras::stats(&ws),
        Command::Verify { revision } => extras::verify(&mut ws, &actor, &prov, &revision),
        Command::Cherry { upstream, head } => ws
            .look(&actor, &prov, |tx| inspect::cherry(tx, &upstream, &head))
            .map_err(Into::into),
        Command::RangeDiff { old, new } => ws
            .look(&actor, &prov, |tx| inspect::range_diff(tx, &old, &new))
            .map_err(Into::into),
        Command::ShowBranch { revisions } => ws
            .look(&actor, &prov, |tx| inspect::show_branch(tx, &revisions))
            .map_err(Into::into),
        Command::Difftool { from, to } => {
            extras::difftool(&mut ws, &actor, &prov, from.as_deref(), to.as_deref())
        }
        Command::Mergetool => extras::mergetool(&mut ws, &actor, &prov),
        Command::Hook(HookCommand::List) => Ok(hooks::list(&ws)),
        Command::Hook(HookCommand::Run { name, args }) => hooks::run_by_hand(&ws, &name, &args),
        Command::Start(args) => ws
            .run(&actor, &prov, name, |tx| start(tx, args))
            .map_err(Into::into),
        Command::Workspaces => {
            ws.snapshot_others(&prov, true)?;
            let activity: BTreeMap<String, String> = ws.last_activity()?.into_iter().collect();
            ws.read(&actor, &prov, |tx| workspaces(tx, &activity))
                .map_err(Into::into)
        }
        Command::Workspace(WorkspaceCommand::Forget { name: target }) => ws
            .run(&actor, &prov, name, |tx| forget(tx, &target))
            .map_err(Into::into),
        Command::Undo => ws.run(&actor, &prov, name, undo).map_err(Into::into),
        Command::Gc {
            auto,
            prune,
            expire_ops,
        } => {
            if auto && !ws.store.needs_gc() {
                return Ok(Output::new(
                    json!({ "skipped": true }),
                    "Nothing to do yet.\n".to_owned(),
                ));
            }
            gc(&mut ws, &prune, &expire_ops)
        }
        Command::Op(OpCommand::Log { limit }) => op_log(&ws, limit),
        Command::Op(OpCommand::Show { id }) => inspect::op_show(&ws, id),
        Command::Op(OpCommand::Restore { id }) => ws
            .run(&actor, &prov, name, |tx| op_restore(tx, id))
            .map_err(Into::into),
        Command::Bookmark(BookmarkCommand::List) => ws
            .look(&actor, &prov, |tx| bookmark(tx, BookmarkCommand::List))
            .map_err(Into::into),
        Command::Bookmark(cmd) => ws
            .run(&actor, &prov, name, |tx| bookmark(tx, cmd))
            .map_err(Into::into),
        Command::Claim { paths, ttl, note } => {
            let ttl = parse_ttl(&ttl)?;
            ws.look(&actor, &prov, |tx| claim(tx, &paths, ttl, note))
                .map_err(Into::into)
        }
        Command::Release { paths } => ws
            .look(&actor, &prov, |tx| release(tx, &paths))
            .map_err(Into::into),
        Command::Claims => ws.look(&actor, &prov, list_claims).map_err(Into::into),
        Command::Config(args) => config(&ws, args),
        Command::Remote {
            command: None | Some(RemoteCommand::List),
            ..
        } => crate::remote::list(&ws),
        Command::Remote {
            command:
                Some(RemoteCommand::Add {
                    name,
                    url,
                    token,
                    git,
                }),
            ..
        } => match bridge::add(&ws, &name, &url, git)? {
            Some(out) => Ok(out),
            None => crate::remote::add(&ws, &name, &url, token),
        },
        Command::Remote {
            command: Some(RemoteCommand::Remove { name }),
            ..
        } => crate::remote::remove(&ws, &name),
        Command::Remote {
            command: Some(RemoteCommand::SetUrl { name, url }),
            ..
        } => {
            ws.config_set(&format!("remote.{name}.url"), &url)?;
            Ok(Output::new(
                json!({ "remote": name, "url": url }),
                format!("{name} -> {url}\n"),
            ))
        }
        Command::Remote {
            command: Some(RemoteCommand::Rename { old, new }),
            ..
        } => crate::remote::rename(&ws, &old, &new),
        Command::Push {
            remote,
            delete: Some(name),
            ..
        } => crate::collab::delete_bookmark(&mut ws, &actor, &prov, &remote, &name),
        Command::Push {
            remote,
            branches,
            bookmark,
            revision,
            delete: None,
            force,
            tags,
            all,
            set_upstream: _,
            no_verify,
        } => {
            if !no_verify {
                hooks::run_required(&ws, "pre-push", &[&remote])?;
            }
            // `HEAD` is the branch you are on, as in `git push origin HEAD`.
            let current = ws.info()?.and_then(|w| w.bookmark);
            let branches: Vec<String> = branches
                .into_iter()
                .map(|b| match (b.as_str(), &current) {
                    ("HEAD", Some(name)) => name.clone(),
                    _ => b,
                })
                .collect();
            if bridge::kind(&ws, &remote)? == bridge::Kind::Hebra {
                return crate::remote::push(
                    &mut ws,
                    &actor,
                    &prov,
                    &remote,
                    crate::remote::PushWhat {
                        bookmark,
                        revision,
                        branches,
                        tags,
                    },
                );
            }
            let mut branches = branches;
            if let Some(name) = bookmark {
                if let Some(rev) = revision {
                    ws.run(&actor, &prov, "bookmark", |tx| {
                        let id = tx.resolve(&rev)?;
                        tx.view.bookmarks.insert(name.clone(), id);
                        Ok(())
                    })?;
                }
                branches.push(name);
            }
            bridge::push(
                &mut ws,
                &actor,
                &prov,
                &remote,
                &branches,
                bridge::PushOptions { force, tags, all },
            )
        }
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
        Command::Pull {
            remote,
            rebase,
            merge,
            ff_only,
            sign,
            ..
        } => {
            let signer = sign::wanted(&ws, &sign);
            bridge::pull(
                &mut ws,
                &actor,
                &prov,
                &remote,
                &bridge::PullMode {
                    rebase,
                    merge,
                    ff_only,
                    signer,
                },
            )
        }
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
        | Command::Heddle(_)
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
            ws.read(&actor, &prov, |tx| git_export(tx, &dir, &branch, &revision))
                .map_err(Into::into)
        }
        Command::Git(GitCommand::Import {
            dir,
            branch,
            bookmark,
            all,
        }) => {
            let dir = cwd.join(dir);
            if all {
                return git_import_all(&mut ws, &actor, &prov, &dir);
            }
            let bookmark = bookmark.unwrap_or_else(|| branch.clone());
            ws.run(&actor, &prov, name, |tx| {
                git_import(tx, &dir, &branch, &bookmark)
            })
            .map_err(Into::into)
        }
    }
}

/// `hb git import --all`: the directory becomes a remote, and every branch
/// and tag comes over, the same as a clone.
fn git_import_all(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    dir: &Path,
) -> Result<Output> {
    let absolute = std::path::absolute(dir)?.display().to_string();
    bridge::add(ws, "git-import", &absolute, true)?;
    let fetched = bridge::fetch(ws, actor, prov, "git-import")?;
    let bookmarks = fetched.bookmarks.clone();
    ws.run(actor, prov, "git import", |tx| {
        for (name, id) in &bookmarks {
            tx.view.bookmarks.entry(name.clone()).or_insert(*id);
        }
        Ok(())
    })?;
    Ok(Output::new(
        json!({ "bookmarks": fetched.bookmarks.keys().collect::<Vec<_>>(), "commits": fetched.objects }),
        format!(
            "Imported {} branch(es) and {} commit(s).\n",
            fetched.bookmarks.len(),
            fetched.objects
        ),
    ))
}

/// The first word of a command, as recorded in the operation log.
fn command_name(command: &Command) -> String {
    let text = format!("{command:?}");
    let word: String = text
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    let mut out = String::new();
    for (i, c) in word.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
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
    tx.own_changes(id)
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

// ---------------------------------------------------------------- commands

/// Who is acting, before any repository exists.
pub(crate) fn fresh_identity() -> (Actor, Provenance) {
    let mut actor = identity::resolve_actor(env, None, configured_user(None));
    if actor.kind == ActorKind::Human {
        actor.email = configured(None, "email");
    }
    (actor, identity::provenance(env, None))
}

fn init(path: &Path) -> Result<Output> {
    std::fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
    let (env_actor, prov) = fresh_identity();
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
    for (bookmark, others) in &tx.view.bookmark_conflicts {
        let list: Vec<String> = others.iter().map(ObjectId::short).collect();
        let _ = writeln!(
            text,
            "Bookmark {bookmark} was moved two ways at once; also at {}. `hb bookmark set` settles it.",
            list.join(", ")
        );
    }
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
        "bookmark_conflicts": tx.view.bookmark_conflicts,
        "warnings": {
            "claimed_by_others": claimed.iter().map(|(p, c)| json!({ "path": p, "claim": c })).collect::<Vec<_>>(),
            "overlapping": overlapping.iter().map(|(p, n, a)| json!({ "path": p, "workspace": n, "actor": a })).collect::<Vec<_>>(),
        },
    });
    Ok(Output::new(json, text))
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

fn new_change(
    tx: &mut Tx<'_>,
    parents: Vec<ObjectId>,
    message: Option<String>,
) -> hebra_core::Result<(ObjectId, Commit)> {
    let parents = tx.simplify_parents(parents)?;
    let (tree, conflicts) = tx.parents_tree(&parents)?;
    let tree_id = tx.store.write_tree(&tree)?;
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
            email: None,
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

fn undo(tx: &mut Tx<'_>) -> hebra_core::Result<Output> {
    let result = tx.undo()?;
    let mut text = format!("Undid operation {} ({}).\n", result.undone, result.command);
    if !result.rebased.is_empty() {
        let _ = writeln!(
            text,
            "Later work kept on top: {}.",
            result.rebased.join(", ")
        );
    }
    if !result.kept.is_empty() {
        let _ = writeln!(
            text,
            "Left as they are (changed in unrelated ways since): {}.",
            result.kept.join(", ")
        );
    }
    Ok(Output::new(json!(result), text))
}

fn gc(ws: &mut Workspace, prune: &str, expire_ops: &str) -> Result<Output> {
    let age = if prune == "now" { 0 } else { parse_ttl(prune)? };
    let prune_before = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::try_from(age).unwrap_or(0));
    let expire = if expire_ops == "never" {
        None
    } else {
        let age = if expire_ops == "now" {
            0
        } else {
            parse_ttl(expire_ops)?
        };
        Some((jiff::Timestamp::now() - jiff::SignedDuration::from_secs(age)).to_string())
    };
    let stats = ws.gc(prune_before, expire.as_deref())?;
    let mirrors = crate::bridge::compact_mirrors(ws)?;
    let mut text = format!(
        "Packed {} object(s); removed {} loose file(s) and {} old pack(s); dropped {} unreachable object(s).\n",
        stats.packed, stats.loose_removed, stats.packs_removed, stats.pruned
    );
    if mirrors > 0 {
        let _ = writeln!(text, "Compacted {mirrors} Git mirror(s).");
    }
    let mut json = json!(stats);
    json["git_mirrors"] = json!(mirrors);
    Ok(Output::new(json, text))
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
    tx.restore(id)?;
    Ok(Output::new(
        json!({ "restored": id }),
        format!("Restored the repository to operation {id}.\n"),
    ))
}

fn bookmark(tx: &mut Tx<'_>, cmd: BookmarkCommand) -> hebra_core::Result<Output> {
    match cmd {
        BookmarkCommand::Set { name, revision } => {
            let id = tx.resolve(&revision)?;
            tx.view.bookmarks.insert(name.clone(), id);
            tx.view.bookmark_conflicts.remove(&name);
            let c = tx.commit(&id)?;
            Ok(Output::new(
                json!({ "bookmark": name, "commit": commit_json(&id, &c) }),
                format!("Bookmark {name} -> {}\n", short_line(&id, &c)),
            ))
        }
        BookmarkCommand::Rename { old, new } => crate::porcelain::bookmark_rename(tx, &old, &new),
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

fn config(ws: &Workspace, args: ConfigArgs) -> Result<Output> {
    if args.global {
        return extras::config_global(
            args.key.as_deref(),
            args.value.as_deref(),
            args.unset,
            args.list,
        );
    }
    let cmd = match (args.command, args.key, args.value) {
        (Some(cmd), _, _) => cmd,
        (None, Some(key), _) if args.unset => ConfigCommand::Unset { key },
        (None, Some(key), Some(value)) => ConfigCommand::Set { key, value },
        (None, Some(key), None) => ConfigCommand::Get { key },
        (None, None, _) => ConfigCommand::List,
    };
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
        ConfigCommand::Unset { key } => {
            ws.config_unset(&key)?;
            Ok(Output::new(
                json!({ "unset": key }),
                format!("Unset {key}\n"),
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
    let head = git(dir, &["rev-parse", "--verify", "--quiet", &reference])?;
    let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    let git_dir = git(dir, &["rev-parse", "--absolute-git-dir"])?;
    let git_dir = std::path::PathBuf::from(String::from_utf8_lossy(&git_dir.stdout).trim());
    if let Some(imported) =
        crate::bridge::import_git_objects(tx, &git_dir, &[(reference.clone(), head)])?
    {
        return git_imported(tx, dir, bookmark, imported.stats, imported.tip);
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
    // One pack instead of a file per object, made durable before the
    // operation is recorded.
    let store = tx.store;
    let imported = store
        .batch(|| hebra_core::gitimport::import(tx, std::io::BufReader::new(stdout), &|_| None))?;
    let (stats, tip) = (imported.stats, imported.tip);
    let out = child.wait_with_output().map_err(io)?;
    if !out.status.success() {
        return Err(git_error("git fast-export", &out));
    }
    git_imported(tx, dir, bookmark, stats, tip)
}

fn git_imported(
    tx: &mut Tx<'_>,
    dir: &Path,
    bookmark: &str,
    stats: hebra_core::gitimport::ImportStats,
    tip: Option<hebra_core::ObjectId>,
) -> hebra_core::Result<Output> {
    let tip =
        tip.ok_or_else(|| Error::InvalidArgument(format!("bookmark `{bookmark}` would be empty")))?;
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
            "  note: {} entries of unknown kinds were skipped",
            stats.skipped_entries
        );
    }
    let _ = writeln!(text, "Start working on it with `hb new {bookmark}`.");
    Ok(Output::new(
        json!({ "bookmark": bookmark, "stats": stats, "tip": commit_json(&tip, &commit) }),
        text,
    ))
}

/// `hb status` must stay instant even if the server is slow or down.
const PRESENCE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(800);

/// If an `origin` remote is configured, shares our presence and adds warnings
/// for files other machines are touching. Best effort: never fails `status`.
fn add_remote_warnings(ws: &mut Workspace, actor: &Actor, prov: &Provenance, out: &mut Output) {
    if ws.config_get("remote.origin.url").ok().flatten().is_none()
        || !crate::remote::presence_due(ws)
    {
        return;
    }
    let changed: Vec<String> = out.json["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["path"].as_str().map(str::to_owned))
        .collect();
    let reply = crate::remote::exchange(ws, actor, prov, "origin", PRESENCE_TIMEOUT);
    crate::remote::presence_answered(ws, reply.is_ok());
    match reply {
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
