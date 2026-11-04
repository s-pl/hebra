use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Hebra: version control built for humans and agents working together.
#[derive(Debug, Parser)]
#[command(name = "hb", version, about, long_about = None, max_term_width = 100)]
pub struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true, env = "HB_JSON", value_parser = clap::builder::BoolishValueParser::new(), default_value_t = false)]
    pub json: bool,

    /// Run as if started in this directory.
    #[arg(short = 'C', long = "cwd", global = true, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Accepted for habit: `hb` never starts a pager.
    #[arg(long, global = true, hide = true)]
    pub no_pager: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a repository here and import the files already present.
    Init {
        /// Directory to initialize (defaults to the current one).
        path: Option<PathBuf>,
        /// Accepted for habit: a repository is the same either way.
        #[arg(long)]
        bare: bool,
    },
    /// Show the working-copy change, its files, conflicts and coordination warnings.
    #[command(visible_alias = "st")]
    Status(StatusArgs),
    /// Show changes as a unified diff.
    Diff(DiffArgs),
    /// Set the description of a change; without `-m`, name a revision after
    /// the nearest tag (`v1.2-3-gabc`), like `git describe`.
    Describe {
        /// Revision to describe.
        #[arg(short, long, default_value = "@")]
        revision: String,
        /// New description.
        #[arg(short, long)]
        message: Option<String>,
        /// Print the bare commit id when no tag is reachable.
        #[arg(long)]
        always: bool,
    },
    /// Describe the working-copy change and start a new empty one on top.
    Commit(CommitArgs),
    /// Start a new change on top of one or more revisions.
    New {
        /// Parent revisions (several create a merge).
        #[arg(default_value = "@")]
        revisions: Vec<String>,
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Show the history of visible changes.
    Log(LogArgs),
    /// Show a change (metadata, provenance and diff), or a file at `rev:path`.
    Show(ShowArgs),
    /// Merge other changes (for example an agent's workspace `name@`) into a new change.
    Merge(MergeArgs),
    /// Mark conflicts in the working-copy change as resolved.
    Resolve {
        /// Paths to mark as resolved (all if omitted).
        paths: Vec<String>,
        /// Settle conflicted regions with our side.
        #[arg(long, conflicts_with = "theirs")]
        ours: bool,
        /// Settle conflicted regions with their side.
        #[arg(long)]
        theirs: bool,
    },
    /// There is no staging area: every file is part of the change already.
    /// Accepted so habits and scripts keep working.
    Add {
        paths: Vec<String>,
        #[arg(short = 'A', long)]
        all: bool,
        #[arg(short, long)]
        update: bool,
    },
    /// Remove files from the working copy.
    Rm {
        #[arg(required = true)]
        paths: Vec<String>,
        /// Keep the files on disk and ignore them from now on.
        #[arg(long)]
        cached: bool,
        #[arg(short, long)]
        recursive: bool,
        #[arg(short, long)]
        force: bool,
    },
    /// Move or rename files.
    Mv {
        /// Source paths, then the destination.
        #[arg(required = true, num_args = 2..)]
        paths: Vec<String>,
        #[arg(short, long)]
        force: bool,
    },
    /// Put files back as they are in another revision (`@-` by default).
    Restore {
        #[arg(required = true)]
        paths: Vec<String>,
        #[arg(short, long, default_value = "@-")]
        source: String,
        /// Accepted for habit: with no staging area there is nothing to do.
        #[arg(long, short = 'S')]
        staged: bool,
        /// Accepted for habit.
        #[arg(long, short = 'W')]
        worktree: bool,
    },
    /// Move the current bookmark and the working copy to another revision.
    Reset {
        /// Where to go (`@-` by default).
        #[arg(default_value = "@-")]
        revision: String,
        /// Discard the files too; they stay recoverable with `hb undo`.
        #[arg(long, conflicts_with_all = ["soft", "mixed"])]
        hard: bool,
        /// Keep the files: what differs shows up as changes.
        #[arg(long)]
        soft: bool,
        /// Same as `--soft`: there is no staging area to reset.
        #[arg(long)]
        mixed: bool,
    },
    /// Delete files new in the working copy; with `-x` also ignored ones.
    Clean {
        /// Only list what would go.
        #[arg(short = 'n', long)]
        dry_run: bool,
        #[arg(short, long)]
        force: bool,
        /// Directories too.
        #[arg(short)]
        d: bool,
        /// Ignored files too.
        #[arg(short)]
        x: bool,
        /// Only ignored files.
        #[arg(short = 'X')]
        only_ignored: bool,
    },
    /// Go to a bookmark or revision; or restore files, with `-- <paths>`.
    Checkout {
        /// Bookmark or revision to go to.
        target: Option<String>,
        /// Create this bookmark and go to it.
        #[arg(short = 'b', short_alias = 'B')]
        create: Option<String>,
        /// Files to restore instead of moving.
        #[arg(last = true)]
        paths: Vec<String>,
    },
    /// Go to a bookmark: new work lands on it, like a checked-out Git branch.
    Switch {
        target: Option<String>,
        /// Create this bookmark and go to it.
        #[arg(short, long)]
        create: Option<String>,
        /// Go to the revision without following any bookmark.
        #[arg(short, long)]
        detach: bool,
    },
    /// List, create, rename or delete branches (bookmarks).
    Branch(BranchArgs),
    /// List, create, delete or verify tags.
    Tag(TagArgs),
    /// Put changes away for later. Alone it saves them, like `stash push`.
    Stash {
        #[command(subcommand)]
        command: Option<StashCommand>,
    },
    /// Move changes onto another revision. Conflicts are recorded, so it
    /// always finishes; there is nothing to continue or abort.
    Rebase(RebaseArgs),
    /// Copy changes onto the working copy.
    #[command(name = "cherry-pick")]
    CherryPick {
        #[arg(required = true)]
        revisions: Vec<String>,
        /// Apply to the working copy without making a change.
        #[arg(short = 'n', long)]
        no_commit: bool,
        /// Record where it was copied from.
        #[arg(short = 'x')]
        record_origin: bool,
        /// For a merge, the parent to compare with (from 1).
        #[arg(short, long)]
        mainline: Option<usize>,
        #[command(flatten)]
        sign: SignArgs,
    },
    /// Add a change that undoes others.
    Revert {
        #[arg(required = true)]
        revisions: Vec<String>,
        #[arg(short = 'n', long)]
        no_commit: bool,
        #[arg(short, long)]
        mainline: Option<usize>,
        #[command(flatten)]
        sign: SignArgs,
    },
    /// Move the changes of one revision into another (its parent by default).
    Squash {
        /// Where the changes come from.
        #[arg(short, long, default_value = "@")]
        from: String,
        /// Where they go (the parent of `--from` by default).
        #[arg(long)]
        into: Option<String>,
        /// Description of the result.
        #[arg(short, long)]
        message: Option<String>,
        /// Move only these paths.
        paths: Vec<String>,
    },
    /// Split a change in two: the given paths first, the rest on top.
    Split {
        #[arg(short, long, default_value = "@")]
        revision: String,
        #[arg(short, long)]
        message: String,
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Remove changes from history; what was on top of them stays.
    Abandon {
        #[arg(required = true)]
        revisions: Vec<String>,
    },
    /// Continue working in an existing change.
    Edit { revision: String },
    /// Rewrite history removing or keeping paths, like `git filter-repo`.
    Filter {
        /// Remove these paths from every commit.
        #[arg(long = "remove")]
        remove: Vec<String>,
        /// Keep only these paths.
        #[arg(long = "keep")]
        keep: Vec<String>,
        /// The tip whose history is rewritten.
        #[arg(short, long, default_value = "@-")]
        revision: String,
    },
    /// Copy a repository: a Hebra server, a Git URL or a local path.
    Clone(CloneArgs),
    /// Download changes and bookmarks without touching local ones.
    Fetch {
        #[arg(default_value = "origin")]
        remote: String,
        /// Accepted for habit: every branch comes.
        branch: Option<String>,
        /// Every remote.
        #[arg(long)]
        all: bool,
        /// Accepted for habit: deleted branches always go.
        #[arg(short, long)]
        prune: bool,
        /// Accepted for habit: tags always come.
        #[arg(short, long)]
        tags: bool,
    },
    /// Who wrote each line, and for which task and prompt.
    #[command(visible_alias = "annotate")]
    Blame {
        path: String,
        /// Revision to look at.
        #[arg(short, long, default_value = "@")]
        revision: String,
        /// Only these lines: `10,20`.
        #[arg(short = 'L')]
        lines: Option<String>,
    },
    /// Search tracked files, in the working copy or a revision.
    Grep(GrepArgs),
    /// Find code by what it says: functions, types and sections ranked by
    /// the words of the question, and the changes whose description matches.
    Search(SearchArgs),
    /// Summarize history by author.
    Shortlog {
        #[arg(short, long)]
        summary: bool,
        #[arg(short = 'e', long)]
        email: bool,
        #[arg(default_value = "@-")]
        revision: String,
    },
    /// List tracked files.
    LsFiles {
        /// Only files changed in the working copy.
        #[arg(short, long)]
        modified: bool,
        /// Only deleted files.
        #[arg(short, long)]
        deleted: bool,
        /// Only ignored files.
        #[arg(short, long)]
        ignored: bool,
        paths: Vec<String>,
    },
    /// List the files of a revision.
    LsTree {
        #[arg(default_value = "@")]
        revision: String,
        #[arg(short, long)]
        recursive: bool,
        /// Only names.
        #[arg(long)]
        name_only: bool,
        path: Option<String>,
    },
    /// Print a file at `rev:path`, or an object.
    #[command(visible_alias = "cat-file")]
    Cat {
        spec: String,
        /// Print the kind of object.
        #[arg(short = 't')]
        kind: bool,
        /// Pretty print (the default).
        #[arg(short = 'p')]
        pretty: bool,
    },
    /// Resolve revisions and repository paths, for scripts.
    RevParse {
        revisions: Vec<String>,
        #[arg(long)]
        show_toplevel: bool,
        #[arg(long)]
        git_dir: bool,
        #[arg(long)]
        is_inside_work_tree: bool,
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        abbrev_ref: bool,
        #[arg(long)]
        short: bool,
    },
    /// List commits, for scripts.
    RevList {
        #[arg(required = true)]
        revisions: Vec<String>,
        #[arg(long)]
        count: bool,
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Best common ancestors of two revisions.
    MergeBase {
        a: String,
        b: String,
        /// Answer with the exit code: is `a` an ancestor of `b`?
        #[arg(long)]
        is_ancestor: bool,
        #[arg(short, long)]
        all: bool,
    },
    /// List every reference: bookmarks, remote bookmarks and tags.
    #[command(visible_alias = "show-ref")]
    ForEachRef {
        /// Only names starting with this.
        pattern: Option<String>,
    },
    /// Name a revision after the bookmarks and tags that contain it.
    NameRev { revision: String },
    /// Find which change introduced a bug, by binary search.
    #[command(subcommand)]
    Bisect(BisectCommand),
    /// Write changes as mail-ready patch files.
    FormatPatch {
        /// `A..B`, or a revision meaning "the changes since it".
        revision: String,
        /// Directory for the files.
        #[arg(short, long, default_value = ".")]
        output_directory: std::path::PathBuf,
        /// Print them instead.
        #[arg(long)]
        stdout: bool,
    },
    /// Apply a patch to the working copy.
    Apply {
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
        /// Only report whether it applies.
        #[arg(long)]
        check: bool,
        /// Apply it backwards.
        #[arg(short = 'R', long)]
        reverse: bool,
    },
    /// Apply mail-formatted patches, one change each.
    Am {
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
        #[command(flatten)]
        sign: SignArgs,
    },
    /// Write the files of a revision to a tar or zip archive.
    Archive {
        #[arg(default_value = "@-")]
        revision: String,
        /// `tar`, `tar.gz` or `zip` (from the output name by default).
        #[arg(long)]
        format: Option<String>,
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
        /// Put everything under this directory.
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Move history as a single file, without a network.
    #[command(subcommand)]
    Bundle(BundleCommand),
    /// Attach notes to changes without rewriting them. Alone it lists them.
    Notes {
        #[command(subcommand)]
        command: Option<NotesCommand>,
    },
    /// Check that every object is intact and reachable.
    Fsck,
    /// Work with part of the files in this workspace.
    #[command(subcommand, name = "sparse", visible_alias = "sparse-checkout")]
    Sparse(SparseCommand),
    /// Git submodules. Alone it shows their status.
    Submodule {
        #[command(subcommand)]
        command: Option<SubmoduleCommand>,
    },
    /// The history of everything done to the repository.
    Reflog {
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Manage workspaces, like `git worktree`.
    #[command(subcommand)]
    Worktree(WorktreeCommand),
    /// How many objects the repository holds and how much space they take.
    #[command(visible_alias = "count-objects")]
    Stats,
    /// Check the signature of a change or tag.
    #[command(visible_alias = "verify-commit")]
    Verify { revision: String },
    /// Commits of `head` not yet in `upstream`, comparing their contents.
    Cherry {
        upstream: String,
        #[arg(default_value = "@-")]
        head: String,
    },
    /// Compare two versions of a series of changes.
    RangeDiff { old: String, new: String },
    /// Show diffs of the changes between bookmarks, side by side.
    ShowBranch { revisions: Vec<String> },
    /// Open a configured tool on each changed file (`diff.tool`).
    Difftool {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    /// Open a configured tool on each conflicted file (`merge.tool`).
    Mergetool,
    /// Run or list repository hooks.
    #[command(subcommand)]
    Hook(HookCommand),
    /// Print the version.
    Version,
    /// Start an isolated workspace for a task, typically one per agent.
    Start(StartArgs),
    /// List workspaces: who is working where, on what, and which files they touch.
    Workspaces,
    /// Manage workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    /// Undo the last operation (everything is recorded in the operation log).
    Undo,
    /// Pack all objects into one file and drop unreachable ones.
    ///
    /// Everything reachable from any recorded operation is kept, so undo and
    /// `op restore` keep working.
    Gc {
        /// Do nothing unless the repository has grown enough to need it.
        #[arg(long)]
        auto: bool,
        /// Drop unreachable objects older than this (`2d`, `12h`, or `now`).
        #[arg(long, default_value = "14d")]
        prune: String,
        /// Forget operations older than this (`90d`, `now`, or `never`). The
        /// most recent ones are always kept.
        #[arg(long, default_value = "90d")]
        expire_ops: String,
    },
    /// Inspect or restore the operation log.
    #[command(subcommand)]
    Op(OpCommand),
    /// Manage bookmarks (named pointers to changes, like `main`).
    #[command(subcommand)]
    Bookmark(BookmarkCommand),
    /// Reserve paths you are about to work on, so others see it before conflicts happen.
    Claim {
        #[arg(required = true)]
        paths: Vec<String>,
        /// How long the claim lasts (`90s`, `30m`, `2h`).
        #[arg(long, default_value = "30m")]
        ttl: String,
        /// What you are doing with these paths.
        #[arg(long)]
        note: Option<String>,
    },
    /// Release claims of this workspace (all if no paths are given).
    Release { paths: Vec<String> },
    /// List active claims across all workspaces.
    Claims,
    /// Read or write configuration (for example `user.name`).
    Config(ConfigArgs),
    /// Serve Hebra to agents over the Model Context Protocol (stdio).
    Mcp,
    /// Interoperate with Git.
    #[command(subcommand)]
    Git(GitCommand),
    /// Manage the remotes this repository syncs with. Alone it lists them.
    Remote {
        #[command(subcommand)]
        command: Option<RemoteCommand>,
        /// Accepted for habit: URLs are always shown.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Send bookmarks (and the changes they point to) to a server.
    ///
    /// Without --bookmark, every bookmark that differs from the server is
    /// pushed. Agents usually publish with
    /// `hb push --bookmark agents/<name>/<task> -r @-`.
    Push {
        /// Remote to push to.
        #[arg(default_value = "origin")]
        remote: String,
        /// Branches to push, as in `hb push origin main`.
        branches: Vec<String>,
        /// Bookmark to push.
        #[arg(short, long)]
        bookmark: Option<String>,
        /// Point the bookmark at this revision first.
        #[arg(short, long, requires = "bookmark")]
        revision: Option<String>,
        /// Delete this bookmark on the server instead.
        #[arg(long, value_name = "BOOKMARK", conflicts_with_all = ["bookmark", "revision"])]
        delete: Option<String>,
        /// Overwrite the remote even if it moved (Git remotes).
        #[arg(short, long, visible_alias = "force-with-lease")]
        force: bool,
        /// Push tags too.
        #[arg(long)]
        tags: bool,
        /// Every bookmark.
        #[arg(long)]
        all: bool,
        /// Accepted for habit: bookmarks always track the remote they go to.
        #[arg(short = 'u', long)]
        set_upstream: bool,
        /// Skip the `pre-push` hook.
        #[arg(long)]
        no_verify: bool,
    },
    /// Ask for a bookmark to be merged, and act on reviews.
    #[command(subcommand)]
    Review(ReviewCommand),
    /// Stay connected: share your presence as you work and see others live.
    Watch {
        #[arg(default_value = "origin")]
        remote: String,
        /// Seconds between presence updates.
        #[arg(long, default_value_t = 5)]
        interval: u64,
    },
    /// Fetch bookmarks and changes from a server; fast-forward local bookmarks.
    Pull {
        #[arg(default_value = "origin")]
        remote: String,
        /// Accepted for habit: every branch comes.
        branch: Option<String>,
        /// Replay local changes on top of what arrived.
        #[arg(long, conflicts_with = "merge")]
        rebase: bool,
        /// Merge what arrived into the current change.
        #[arg(long)]
        merge: bool,
        /// Fail if the current bookmark has diverged.
        #[arg(long)]
        ff_only: bool,
        /// Accepted for habit: merging is what `--merge` does.
        #[arg(long)]
        no_rebase: bool,
        #[command(flatten)]
        sign: SignArgs,
    },
    /// Who is working on the server's repository right now, from any machine.
    Presence {
        #[arg(default_value = "origin")]
        remote: String,
    },
    /// Check the server's transparency log: hash chain, signed head, and that
    /// the bookmarks it shows are the ones it recorded.
    VerifyLog {
        #[arg(default_value = "origin")]
        remote: String,
        /// Fail unless the log is signed by this public key (hex).
        #[arg(long)]
        key: Option<String>,
    },
    /// Tasks: work items with acceptance criteria that agents claim with a lease.
    #[command(subcommand)]
    Task(TaskCommand),
    /// Checks run by runners for pushes and reviews (`.hb/checks.toml`).
    #[command(subcommand)]
    Checks(ChecksCommand),
    /// Sign in to a server (shows a code to approve in the browser).
    Login {
        /// Server URL, e.g. https://hebra.dev (default: this repository's origin).
        server: Option<String>,
    },
    /// Sign out of a server and revoke the stored mandate.
    Logout { server: Option<String> },
    /// Who you are on a server.
    Whoami { server: Option<String> },
    /// Mandates: hand narrowed credentials to agents.
    #[command(subcommand)]
    Mandate(MandateCommand),
    /// Runners: run checks on your own machines.
    #[command(subcommand)]
    Runner(RunnerCommand),
    /// Heddle™, build intelligence: simulate policies, and start and stop
    /// runner machines for a server's queue.
    #[command(subcommand)]
    Heddle(HeddleCommand),
    /// Organizations on a server.
    #[command(subcommand)]
    Org(OrgCommand),
    /// Agent accounts that belong to you or an organization.
    #[command(subcommand)]
    Agent(AgentCommand),
    /// Repositories on a server.
    #[command(subcommand)]
    Repo(RepoCommand),
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// List tasks (`open`, `ready`, `claimed`, `done`, `cancelled`, `all`).
    List {
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Create a task.
    Create {
        title: String,
        #[arg(long)]
        body: Option<String>,
        /// An acceptance criterion (repeat for several).
        #[arg(long = "accept")]
        acceptance: Vec<String>,
        #[arg(long = "label")]
        labels: Vec<String>,
        /// Make it a sub-task of this task.
        #[arg(long)]
        parent: Option<i64>,
        /// Only ready once these tasks are done.
        #[arg(long = "after")]
        after: Vec<i64>,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Show a task.
    Show {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Claim a task for a while (claiming again renews it).
    Claim {
        id: i64,
        /// Minutes to hold it.
        #[arg(long = "for", default_value_t = 30)]
        minutes: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Give a claimed task back.
    Release {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Mark a task done (merging a review that names it does this too).
    Done {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Cancel a task.
    Cancel {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Reopen a closed task.
    Reopen {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ChecksCommand {
    /// Checks of a commit or bookmark (recent runs when omitted).
    List {
        revision: Option<String>,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Print a run's log.
    Log {
        id: String,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Run checks again on a commit or bookmark.
    Rerun {
        revision: String,
        /// Only this check.
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Let checks waiting on a contribution run (maintainers).
    Approve {
        revision: String,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum MandateCommand {
    /// Narrow your mandate for an agent and print it (offline).
    Grant {
        /// Agent name (recorded as the author of its work).
        #[arg(long)]
        agent: String,
        /// Limit to these repositories `owner/name` (default: this one).
        #[arg(long = "repo")]
        repos: Vec<String>,
        /// Operations: read, push, review, merge, task, admin, run
        /// (default: read, push, review, task).
        #[arg(long = "op")]
        operations: Vec<String>,
        /// Bookmarks it may move (default: agents/<agent>/*).
        #[arg(long = "bookmark")]
        bookmarks: Vec<String>,
        /// Lifetime: `8h`, `30m`, `2d`.
        #[arg(long, default_value = "8h")]
        ttl: String,
        /// What the agent is for (shown in provenance).
        #[arg(long)]
        purpose: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Your mandates on a server.
    List {
        #[arg(long)]
        server: Option<String>,
    },
    /// Revoke a mandate (and every mandate derived from it).
    Revoke {
        id: String,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunnerCommand {
    /// Register this machine as a runner for a person or organization.
    Register {
        /// Whose runner it is (with your sign-in; not needed with --token).
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long)]
        server: Option<String>,
        /// A one-time runner token (`hb runner token`), for machines nobody
        /// signs in on: a VPS, a container an orchestrator starts.
        #[arg(long, env = "HB_RUNNER_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// The provider's id of this machine (orchestrated runners).
        #[arg(long)]
        machine: Option<String>,
        /// Also start it as a service that comes back with the machine.
        #[arg(long)]
        install: bool,
    },
    /// Take and run checks until stopped.
    Start {
        name: String,
        /// Run at most one check, then exit.
        #[arg(long)]
        once: bool,
        /// Take one check, then remove this runner (machines that go away).
        #[arg(long)]
        ephemeral: bool,
    },
    /// Run a registered runner as a service (systemd, launchd, or a
    /// Windows scheduled task).
    Install { name: String },
    /// Make a one-time token a machine registers with.
    Token {
        #[arg(long)]
        owner: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        /// Runners made with it take one check and go.
        #[arg(long)]
        ephemeral: bool,
        /// How many machines may use it.
        #[arg(long, default_value_t = 1)]
        uses: i64,
        /// How long it stays valid (`30m`, `24h`, `7d`).
        #[arg(long, default_value = "1h")]
        ttl: String,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum HeddleCommand {
    /// Compare policies on a trace of jobs: cost, waits, machines.
    Simulate {
        /// A JSON trace: `[{ "at": 0, "secs": 90, "labels": ["linux"] }, …]`.
        #[arg(long)]
        trace: Option<String>,
        /// A made-up workload when there is no trace: team, agents, steady.
        #[arg(long, default_value = "team")]
        workload: String,
        #[arg(long, default_value_t = 5)]
        days: u32,
        /// Machine types and prices (TOML); an illustrative one by default.
        #[arg(long)]
        catalog: Option<String>,
        /// cheap, balanced or fast.
        #[arg(long, default_value = "balanced")]
        goal: String,
        /// What an hour of someone waiting is worth, in the catalog's
        /// currency (overrides the goal's).
        #[arg(long)]
        time_value: Option<f64>,
        #[arg(long, default_value_t = 20)]
        max_machines: usize,
        #[arg(long, default_value_t = 7)]
        seed: u64,
    },
    /// Start and stop runner machines for an owner's queue (Docker
    /// containers stand in for virtual machines).
    Up {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        catalog: Option<String>,
        /// The runner image (deploy/docker/runner.Dockerfile).
        #[arg(long, default_value = "hebra-runner")]
        image: String,
        #[arg(long, default_value_t = 4)]
        max_machines: usize,
        /// Most the running machines may cost per hour together.
        #[arg(long, default_value_t = 5.0)]
        max_spend: f64,
        /// cheap, balanced or fast.
        #[arg(long, default_value = "balanced")]
        goal: String,
        #[arg(long)]
        time_value: Option<f64>,
        /// Say what it would do, once, and do nothing.
        #[arg(long)]
        dry_run: bool,
        /// Stop after this many seconds (default: until interrupted).
        #[arg(long, default_value_t = 0)]
        duration: u64,
    },
    /// Stop every machine Heddle started for an owner.
    Down {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum OrgCommand {
    /// Create an organization you own.
    Create {
        handle: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Add a person or change their role (owner, admin, member, billing).
    Add {
        org: String,
        person: String,
        #[arg(long, default_value = "member")]
        role: String,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Create an agent account and print its first mandate.
    Create {
        handle: String,
        /// Person or organization it belongs to (default: you).
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    /// Create a repository `owner/name` on a server.
    Create {
        full_name: String,
        #[arg(long)]
        public: bool,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReviewCommand {
    /// Ask for BOOKMARK to be merged into TARGET (default main).
    Open {
        bookmark: String,
        #[arg(short, long)]
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long, default_value = "main")]
        target: String,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// List open reviews (or all with --all).
    List {
        #[arg(long)]
        all: bool,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Approve and merge a review (people only for protected bookmarks).
    Merge {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Close a review without merging.
    Close {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Show a review: changes, approvals, checks and comments.
    Show {
        id: i64,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Comment on a review (on a line with --path and --line).
    Comment {
        id: i64,
        body: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        line: Option<i64>,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Approve a review (people only), or request changes.
    Approve {
        id: i64,
        #[arg(long)]
        changes: bool,
        #[arg(long)]
        note: Option<String>,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// Add a server, e.g. `hb remote add origin http://localhost:4600/acme`.
    Add {
        name: String,
        url: String,
        /// Access token (or set HB_TOKEN).
        #[arg(long)]
        token: Option<String>,
        /// Treat it as a Git repository (detected from the URL otherwise).
        #[arg(long)]
        git: bool,
    },
    /// List remotes.
    #[command(visible_alias = "show")]
    List,
    /// Remove a remote.
    #[command(visible_alias = "rm")]
    Remove { name: String },
    /// Change a remote's URL.
    SetUrl { name: String, url: String },
    /// Rename a remote.
    Rename { old: String, new: String },
}

#[derive(Debug, Subcommand)]
pub enum GitCommand {
    /// Export history into a Git repository (created bare if missing).
    ///
    /// Exports are deterministic: running it again only moves the branch.
    /// Provenance is kept as `Hebra-*` trailers in each commit message.
    Export {
        /// Path of the Git repository to write to.
        dir: PathBuf,
        /// Branch to create or move.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Revision to export, with all its ancestors.
        #[arg(short, long, default_value = "@-")]
        revision: String,
    },
    /// Import the history of a Git branch as a bookmark.
    ///
    /// Deterministic, and commits exported by Hebra get their change ids and
    /// provenance back from their trailers.
    Import {
        /// Path of the Git repository to read from.
        dir: PathBuf,
        /// Git branch to import.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Bookmark to create (defaults to the branch name).
        #[arg(long)]
        bookmark: Option<String>,
        /// Every branch and tag instead.
        #[arg(long, conflicts_with_all = ["branch", "bookmark"])]
        all: bool,
    },
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// One line per file.
    #[arg(short, long)]
    pub short: bool,
    /// Same, in a format stable for scripts.
    #[arg(long)]
    pub porcelain: bool,
    /// Also name the bookmark.
    #[arg(short = 'b', long)]
    pub branch: bool,
}

/// Whether to sign what a command makes.
#[derive(Debug, Args)]
pub struct SignArgs {
    /// Sign the result, with this key if given (see `commit.gpgsign`).
    #[arg(short = 'S', long, value_name = "KEY", num_args = 0..=1, require_equals = true)]
    pub gpg_sign: Option<Option<String>>,
    /// Do not sign, whatever `commit.gpgsign` says.
    #[arg(long, conflicts_with = "gpg_sign")]
    pub no_gpg_sign: bool,
}

#[derive(Debug, Args)]
pub struct CommitArgs {
    /// Description; several become paragraphs. Without one, opens the editor.
    #[arg(short, long)]
    pub message: Vec<String>,
    /// Read the description from a file.
    #[arg(short = 'F', long, value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// Fold the changes into the previous change instead.
    #[arg(long)]
    pub amend: bool,
    /// Accepted for habit: there is nothing to stage.
    #[arg(short, long)]
    pub all: bool,
    /// Accepted for habit: empty changes are always allowed.
    #[arg(long)]
    pub allow_empty: bool,
    /// Accepted for habit.
    #[arg(long)]
    pub no_edit: bool,
    /// Record someone else as author: `Name <email>`.
    #[arg(long)]
    pub author: Option<String>,
    /// When the work was written.
    #[arg(long)]
    pub date: Option<String>,
    /// Skip the `pre-commit` and `commit-msg` hooks.
    #[arg(long)]
    pub no_verify: bool,
    #[command(flatten)]
    pub sign: SignArgs,
    /// Make a change to fold into this revision with `rebase --autosquash`.
    #[arg(long, value_name = "REV")]
    pub fixup: Option<String>,
    /// Like `--fixup`, keeping this change's description too.
    #[arg(long, value_name = "REV")]
    pub squash: Option<String>,
    /// The task this work is for (`#12`); merging its review closes it.
    /// Overrides HB_TASK.
    #[arg(long)]
    pub task: Option<String>,
    /// Choose hunk by hunk what goes in; the rest stays in the working copy.
    #[arg(short, long)]
    pub patch: bool,
    /// Commit only these paths; the rest stays in the working copy.
    pub paths: Vec<String>,
}

#[derive(Debug, Args)]
pub struct LogArgs {
    /// A revision, `A..B` or `A...B`. All visible changes by default. A path
    /// that is not a revision limits the history to that path.
    pub revisions: Vec<String>,
    /// Maximum number of changes to show.
    #[arg(short = 'n', long, default_value_t = 20)]
    pub limit: usize,
    /// Every commit reachable from anything, without a limit.
    #[arg(long)]
    pub all: bool,
    /// One line per change.
    #[arg(long)]
    pub oneline: bool,
    /// Draw the graph.
    #[arg(long)]
    pub graph: bool,
    /// Include the diff.
    #[arg(short, long)]
    pub patch: bool,
    /// Include changed files and line counts.
    #[arg(long)]
    pub stat: bool,
    #[arg(long)]
    pub name_only: bool,
    #[arg(long)]
    pub name_status: bool,
    /// Only by this person or agent (substring).
    #[arg(long)]
    pub author: Option<String>,
    /// Only after this date (`2026-01-02`, `2w`, `3d`).
    #[arg(long, visible_alias = "after")]
    pub since: Option<String>,
    /// Only before this date.
    #[arg(long, visible_alias = "before")]
    pub until: Option<String>,
    /// Only changes whose description matches this pattern.
    #[arg(long)]
    pub grep: Option<String>,
    /// Ignore case in `--grep`.
    #[arg(short = 'i', long)]
    pub regexp_ignore_case: bool,
    /// Only changes that added or removed a line matching this pattern.
    #[arg(short = 'G')]
    pub line_regex: Option<String>,
    /// Only changes that changed how many times this text appears.
    #[arg(short = 'S')]
    pub pickaxe: Option<String>,
    #[arg(long)]
    pub merges: bool,
    #[arg(long)]
    pub no_merges: bool,
    /// Follow only the first parent of merges.
    #[arg(long)]
    pub first_parent: bool,
    /// Oldest first.
    #[arg(long)]
    pub reverse: bool,
    /// Accepted for habit: labels are always shown.
    #[arg(long)]
    pub decorate: bool,
    /// Accepted for habit: commit ids are always short.
    #[arg(long)]
    pub abbrev_commit: bool,
    /// Keep following a file across renames.
    #[arg(long)]
    pub follow: bool,
    /// Custom format: %H %h %s %b %an %ae %ad %ar %cn %d %T %P.
    #[arg(long, visible_alias = "pretty", value_name = "FORMAT")]
    pub format: Option<String>,
    /// Limit to these paths.
    #[arg(last = true)]
    pub paths: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The working copy if it has changes, otherwise the last change.
    pub revision: Option<String>,
    #[arg(long)]
    pub stat: bool,
    #[arg(long)]
    pub name_only: bool,
    #[arg(long)]
    pub name_status: bool,
    /// Without the diff.
    #[arg(short = 's', long)]
    pub no_patch: bool,
    /// One line.
    #[arg(long)]
    pub oneline: bool,
    /// Custom format, as in `hb log`.
    #[arg(long, visible_alias = "pretty", value_name = "FORMAT")]
    pub format: Option<String>,
}

#[derive(Debug, Args)]
pub struct MergeArgs {
    #[arg(required_unless_present_any = ["abort", "resume"])]
    pub revisions: Vec<String>,
    /// A merge never stops, so this only says what to do: `hb undo`.
    #[arg(long)]
    pub abort: bool,
    /// A merge never stops, so there is nothing to continue.
    #[arg(long = "continue")]
    pub resume: bool,
    /// Accepted for habit: nothing opens an editor.
    #[arg(long)]
    pub no_edit: bool,
    #[arg(short, long)]
    pub message: Option<String>,
    /// Keep a merge even if it could fast-forward.
    #[arg(long)]
    pub no_ff: bool,
    /// Fail unless it can fast-forward.
    #[arg(long)]
    pub ff_only: bool,
    /// Bring the files in as changes, without a merge in history.
    #[arg(long)]
    pub squash: bool,
    /// `ours` keeps our files; `recursive` is the default.
    #[arg(short = 's', long)]
    pub strategy: Option<String>,
    /// `ours` or `theirs`: settle conflicted regions with that side.
    #[arg(short = 'X', long)]
    pub strategy_option: Option<String>,
    /// `hb commit` finishes a merge, so that is where to sign it.
    #[command(flatten)]
    pub sign: SignArgs,
}

#[derive(Debug, Args)]
pub struct BranchArgs {
    /// Branch to create, or its new name with `-m`.
    pub name: Option<String>,
    /// Where it starts (`@-` by default).
    pub start: Option<String>,
    /// Delete it.
    #[arg(short = 'd', long, visible_short_alias = 'D')]
    pub delete: bool,
    /// Rename: `-m old new`, or the current one with `-m new`.
    #[arg(short = 'm', long = "move", visible_short_alias = 'M')]
    pub rename: bool,
    /// Move it if it already exists.
    #[arg(short, long)]
    pub force: bool,
    /// List remote ones too.
    #[arg(short, long)]
    pub all: bool,
    /// List only remote ones.
    #[arg(short, long)]
    pub remotes: bool,
    /// Those that contain this revision.
    #[arg(long, value_name = "REV")]
    pub contains: Option<String>,
    /// Those already merged into this revision.
    #[arg(long, value_name = "REV", num_args = 0..=1, default_missing_value = "@-")]
    pub merged: Option<String>,
    /// Those not yet merged into this revision.
    #[arg(long, value_name = "REV", num_args = 0..=1, default_missing_value = "@-")]
    pub no_merged: Option<String>,
    /// Print the current one.
    #[arg(long)]
    pub show_current: bool,
    /// Accepted for habit: the last change is always shown.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Args)]
pub struct TagArgs {
    /// Tag to create (or to delete, verify or look up).
    pub name: Option<String>,
    /// What it points at (`@-` by default).
    pub revision: Option<String>,
    /// Annotated tag, with a description.
    #[arg(short, long)]
    pub annotate: bool,
    #[arg(short, long)]
    pub message: Option<String>,
    /// Sign it.
    #[arg(short, long)]
    pub sign: bool,
    #[arg(short, long)]
    pub delete: bool,
    /// Replace it if it exists.
    #[arg(short, long)]
    pub force: bool,
    /// List tags, optionally matching a pattern.
    #[arg(short, long)]
    pub list: bool,
    /// Those that contain this revision.
    #[arg(long, value_name = "REV")]
    pub contains: Option<String>,
    /// Check its signature.
    #[arg(short, long)]
    pub verify: bool,
}

#[derive(Debug, Subcommand)]
pub enum StashCommand {
    /// Put the working copy's changes away and leave it clean.
    #[command(visible_alias = "save")]
    Push {
        #[arg(short, long)]
        message: Option<String>,
        /// Accepted for habit: new files are always included.
        #[arg(short = 'u', long)]
        include_untracked: bool,
        /// Only these paths.
        paths: Vec<String>,
    },
    /// Apply the latest stash (or `stash@{n}`) and remove it.
    Pop {
        stash: Option<String>,
    },
    /// Apply a stash and keep it.
    Apply {
        stash: Option<String>,
    },
    List,
    /// Show what a stash holds.
    Show {
        stash: Option<String>,
    },
    Drop {
        stash: Option<String>,
    },
    Clear,
    /// Start a branch from where the stash was made, with its changes.
    Branch {
        name: String,
        stash: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct RebaseArgs {
    /// Destination, as in `git rebase main`.
    pub upstream: Option<String>,
    /// Destination.
    #[arg(short, long, visible_alias = "onto")]
    pub destination: Option<String>,
    /// Move this revision and everything on top of it.
    #[arg(short, long, conflicts_with_all = ["revision", "branch"])]
    pub source: Option<String>,
    /// Move only this revision; what was on top stays where it is.
    #[arg(short, long, conflicts_with = "branch")]
    pub revision: Option<String>,
    /// Move the whole branch of this revision (`@` by default).
    #[arg(short, long)]
    pub branch: Option<String>,
    /// Edit the list of steps (`pick`, `reword`, `squash`, `fixup`, `drop`).
    #[arg(short, long)]
    pub interactive: bool,
    /// Take the steps from this file instead of an editor.
    #[arg(long, value_name = "FILE")]
    pub todo: Option<PathBuf>,
    /// Fold `fixup!` and `squash!` changes into their targets.
    #[arg(long)]
    pub autosquash: bool,
    /// Only for habit: a rebase never stops, so this does nothing.
    #[arg(long = "continue")]
    pub resume: bool,
    /// Only for habit: use `hb undo`.
    #[arg(long)]
    pub abort: bool,
    #[command(flatten)]
    pub sign: SignArgs,
}

#[derive(Debug, Args)]
pub struct CloneArgs {
    /// A Hebra server URL, a Git URL or a local path.
    pub url: String,
    pub directory: Option<PathBuf>,
    /// Name of the remote.
    #[arg(short, long, default_value = "origin")]
    pub origin: String,
    /// Start on this branch.
    #[arg(short, long)]
    pub branch: Option<String>,
    /// Only this many commits of history (Git remotes).
    #[arg(long)]
    pub depth: Option<u32>,
    /// Only one branch.
    #[arg(long)]
    pub single_branch: bool,
    /// Do not write the files.
    #[arg(short, long)]
    pub no_checkout: bool,
    /// Fetch submodules too.
    #[arg(long, visible_alias = "recursive")]
    pub recurse_submodules: bool,
    /// Accepted for habit, like `--no-checkout`: a repository is the same.
    #[arg(long)]
    pub bare: bool,
    /// Download file contents on demand: `blob:none` (Hebra servers).
    #[arg(long, value_name = "FILTER")]
    pub filter: Option<String>,
    /// Treat the URL as a Git repository.
    #[arg(long)]
    pub git: bool,
    /// Access token.
    #[arg(long)]
    pub token: Option<String>,
}

#[derive(Debug, Args)]
pub struct GrepArgs {
    pub pattern: String,
    /// Revision to search; the working copy by default.
    pub revision: Option<String>,
    #[arg(short, long)]
    pub ignore_case: bool,
    /// Whole words.
    #[arg(short, long)]
    pub word_regexp: bool,
    /// Treat the pattern as plain text.
    #[arg(short = 'F', long)]
    pub fixed_strings: bool,
    /// Accepted for habit: the numbers are always there in `--json`.
    #[arg(short = 'n', long)]
    pub line_number: bool,
    /// Only file names.
    #[arg(short = 'l', long)]
    pub files_with_matches: bool,
    /// Matches per file.
    #[arg(short, long)]
    pub count: bool,
    /// Lines that do not match.
    #[arg(short = 'v', long)]
    pub invert_match: bool,
    /// Limit to these paths.
    #[arg(last = true)]
    pub paths: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Words to look for: `hb search parse date`.
    #[arg(required = true, num_args = 1..)]
    pub query: Vec<String>,
    /// Revision to search; the working copy by default.
    #[arg(short, long)]
    pub revision: Option<String>,
    /// How many results.
    #[arg(short = 'n', long, default_value_t = 10)]
    pub limit: usize,
    /// Only code, not change descriptions.
    #[arg(long)]
    pub code_only: bool,
    /// Limit to these paths.
    #[arg(last = true)]
    pub paths: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum BisectCommand {
    /// Start: `hb bisect start [<bad> [<good>]]`.
    Start {
        bad: Option<String>,
        good: Option<String>,
    },
    /// Mark the current revision (or the given one) as bad.
    #[command(visible_alias = "new")]
    Bad { revision: Option<String> },
    /// Mark it as good.
    #[command(visible_alias = "old")]
    Good { revision: Option<String> },
    /// It cannot be tested: try a neighbour.
    Skip { revision: Option<String> },
    /// Test with a command: 0 good, 125 skip, anything else bad.
    Run {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Show the progress.
    Log,
    /// Finish and go back to where it started.
    Reset,
}

#[derive(Debug, Subcommand)]
pub enum BundleCommand {
    /// Write history into a file.
    Create {
        file: PathBuf,
        /// Bookmarks to include (all by default).
        bookmarks: Vec<String>,
    },
    /// Load a bundle: its bookmarks land in `bundle` as a remote.
    Unbundle { file: PathBuf },
    /// List what a bundle holds.
    ListHeads { file: PathBuf },
    /// Check that a bundle is intact.
    Verify { file: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum NotesCommand {
    /// Add or replace the note of a revision.
    Add {
        #[arg(default_value = "@-")]
        revision: String,
        #[arg(short, long)]
        message: String,
    },
    /// Show the note of a revision.
    Show {
        #[arg(default_value = "@-")]
        revision: String,
    },
    List,
    #[command(visible_alias = "rm")]
    Remove {
        #[arg(default_value = "@-")]
        revision: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SparseCommand {
    /// Only keep these paths in this workspace.
    Set {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Add paths to the ones kept.
    Add {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    List,
    /// Bring every file back.
    Disable,
}

#[derive(Debug, Subcommand)]
pub enum SubmoduleCommand {
    /// Add a submodule at a path.
    Add { url: String, path: String },
    /// Fetch submodules and check out the pinned commits.
    Update {
        #[arg(long)]
        init: bool,
        #[arg(long)]
        recursive: bool,
    },
    /// List submodules and whether they match the pinned commit.
    Status,
    /// Run a command in each.
    Foreach {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommand {
    /// Add a workspace at a path.
    Add {
        path: PathBuf,
        /// Where it starts.
        #[arg(default_value = "@-")]
        revision: String,
    },
    List,
    /// Stop tracking a workspace and delete its directory.
    Remove {
        name: String,
        /// Leave the directory on disk.
        #[arg(long)]
        keep: bool,
    },
    /// Forget workspaces whose directory is gone.
    Prune,
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Hooks in this repository.
    List,
    /// Run one by hand.
    Run { name: String, args: Vec<String> },
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
    /// In every repository of this person.
    #[arg(long, global = true)]
    pub global: bool,
    /// Key, as in `git config user.name`.
    pub key: Option<String>,
    /// Value to set.
    pub value: Option<String>,
    /// Remove the key.
    #[arg(long)]
    pub unset: bool,
    /// List everything.
    #[arg(short, long)]
    pub list: bool,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Show the changes made by this revision.
    #[arg(short, long, conflicts_with_all = ["from", "to"])]
    pub revision: Option<String>,
    /// Compare from this revision...
    #[arg(long)]
    pub from: Option<String>,
    /// ...to this revision (defaults to `@`).
    #[arg(long)]
    pub to: Option<String>,
    /// Only show a summary of changed files and line counts.
    #[arg(long)]
    pub stat: bool,
    /// Accepted for habit: there is no staging area to compare with.
    #[arg(long, visible_alias = "staged")]
    pub cached: bool,
    /// Only file names.
    #[arg(long)]
    pub name_only: bool,
    /// File names with their status.
    #[arg(long)]
    pub name_status: bool,
    /// Lines of context.
    #[arg(short = 'U', long = "unified", default_value_t = 3)]
    pub context: usize,
    /// Show renames as a delete and an add.
    #[arg(long)]
    pub no_renames: bool,
    /// Revisions, as in `git diff A B`, `A..B` or `A...B`; a path that is
    /// not a revision limits the diff to it.
    pub targets: Vec<String>,
    /// Limit to these paths.
    #[arg(last = true)]
    pub paths: Vec<String>,
}

#[derive(Debug, Args)]
pub struct StartArgs {
    /// What this workspace is for; becomes the change description.
    pub task: String,
    /// Workspace name (derived from the task by default).
    #[arg(long)]
    pub name: Option<String>,
    /// Name of the agent that will work here.
    #[arg(long)]
    pub agent: Option<String>,
    /// Person the agent works for (defaults to you).
    #[arg(long = "for", value_name = "PERSON")]
    pub on_behalf_of: Option<String>,
    /// Revision to start from.
    #[arg(long, default_value = "@-")]
    pub from: String,
    /// Directory for the workspace (defaults to `.hebra/workspaces/<name>`).
    #[arg(long)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Stop tracking a workspace (its directory is left on disk).
    Forget { name: String },
}

#[derive(Debug, Subcommand)]
pub enum OpCommand {
    /// List recent operations.
    Log {
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Restore the repository to the state after an operation.
    Restore { id: i64 },
    /// Show the changes an operation made.
    Show { id: i64 },
}

#[derive(Debug, Subcommand)]
pub enum BookmarkCommand {
    /// Create or move a bookmark.
    Set {
        name: String,
        #[arg(short, long, default_value = "@-")]
        revision: String,
    },
    /// List bookmarks.
    List,
    /// Delete a bookmark.
    Delete { name: String },
    /// Rename a bookmark.
    Rename { old: String, new: String },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Get { key: String },
    Set { key: String, value: String },
    Unset { key: String },
    List,
}
