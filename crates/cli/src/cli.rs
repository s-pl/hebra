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
