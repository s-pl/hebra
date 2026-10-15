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

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a repository here and import the files already present.
    Init {
        /// Directory to initialize (defaults to the current one).
        path: Option<PathBuf>,
    },
    /// Show the working-copy change, its files, conflicts and coordination warnings.
    #[command(visible_alias = "st")]
    Status,
    /// Show changes as a unified diff.
    Diff(DiffArgs),
    /// Set the description of a change.
    Describe {
        /// Revision to describe.
        #[arg(short, long, default_value = "@")]
        revision: String,
        /// New description.
        #[arg(short, long)]
        message: String,
    },
    /// Describe the working-copy change and start a new empty one on top.
    Commit {
        #[arg(short, long)]
        message: String,
    },
    /// Start a new change on top of one or more revisions.
    New {
        /// Parent revisions (several create a merge).
        #[arg(default_value = "@")]
        revisions: Vec<String>,
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Show the history of visible changes.
    Log {
        /// Maximum number of changes to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a change: metadata, provenance and diff.
    Show {
        #[arg(default_value = "@")]
        revision: String,
    },
    /// Merge other changes (for example an agent's workspace `name@`) into a new change.
    Merge {
        #[arg(required = true)]
        revisions: Vec<String>,
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Mark conflicts in the working-copy change as resolved.
    Resolve {
        /// Paths to mark as resolved (all if omitted).
        paths: Vec<String>,
    },
    /// Start an isolated workspace for a task, typically one per agent.
    Start(StartArgs),
    /// List workspaces: who is working where, on what, and which files they touch.
    Workspaces,
    /// Manage workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    /// Undo the last operation (everything is recorded in the operation log).
    Undo,
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
    /// Read or write repository configuration (for example `user.name`).
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Serve Hebra to agents over the Model Context Protocol (stdio).
    Mcp,
    /// Interoperate with Git.
    #[command(subcommand)]
    Git(GitCommand),
    /// Manage Hebra servers this repository syncs with.
    #[command(subcommand)]
    Remote(RemoteCommand),
    /// Send bookmarks (and the changes they point to) to a server.
    ///
    /// Without --bookmark, every bookmark that differs from the server is
    /// pushed. Agents usually publish with
    /// `hb push --bookmark agents/<name>/<task> -r @-`.
    Push {
        /// Remote to push to.
        #[arg(default_value = "origin")]
        remote: String,
        /// Bookmark to push.
        #[arg(short, long)]
        bookmark: Option<String>,
        /// Point the bookmark at this revision first.
        #[arg(short, long, requires = "bookmark")]
        revision: Option<String>,
        /// Delete this bookmark on the server instead.
        #[arg(long, value_name = "BOOKMARK", conflicts_with_all = ["bookmark", "revision"])]
        delete: Option<String>,
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
    },
    /// Who is working on the server's repository right now, from any machine.
    Presence {
        #[arg(default_value = "origin")]
        remote: String,
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
        #[arg(long)]
        owner: String,
        #[arg(long)]
        name: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Take and run checks until stopped.
    Start {
        name: String,
        /// Run at most one check, then exit.
        #[arg(long)]
        once: bool,
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
    },
    /// List remotes.
    List,
    /// Remove a remote.
    Remove { name: String },
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
    },
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
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Get { key: String },
    Set { key: String, value: String },
    List,
}
