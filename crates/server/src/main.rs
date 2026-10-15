//! `hebra-server`: run the server and manage repositories and tokens.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use hebra_server::{Server, TokenInfo};

#[derive(Parser)]
#[command(
    name = "hebra-server",
    version,
    about = "Host Hebra repositories for people and agents"
)]
struct Cli {
    /// Directory holding repositories and the server database.
    #[arg(long, global = true, env = "HEBRA_DATA", default_value = "hebra-data")]
    data: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP server.
    Serve {
        #[arg(long, env = "HEBRA_ADDR", default_value = "127.0.0.1:4600")]
        addr: String,
    },
    /// Manage repositories.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Manage access tokens.
    #[command(subcommand)]
    Token(TokenCommand),
}

#[derive(Subcommand)]
enum RepoCommand {
    /// Create a repository.
    Create {
        name: String,
        /// Bookmarks only people can move (agents push elsewhere).
        #[arg(long = "protect", default_values_t = vec!["main".to_owned()])]
        protect: Vec<String>,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Create a token. The secret is printed once.
    Create {
        /// Who uses it (a person or an agent name).
        #[arg(long)]
        name: String,
        /// The token belongs to an agent (it can never move protected bookmarks).
        #[arg(long)]
        agent: bool,
        /// Person the agent works for.
        #[arg(long = "for")]
        on_behalf_of: Option<String>,
        /// Bookmarks it may move (`*` wildcard). Defaults to everything for
        /// people and `agents/<name>/*` for agents.
        #[arg(long)]
        allow: Vec<String>,
        /// Can create repositories.
        #[arg(long)]
        admin: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let server = Server::open(&cli.data)?;
    match cli.command {
        Command::Serve { addr } => {
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            println!(
                "hebra-server listening on http://{addr} (data: {})",
                cli.data.display()
            );
            hebra_server::serve(Arc::new(server), listener).await?;
        }
        Command::Repo(RepoCommand::Create { name, protect }) => {
            server.create_repo(&name, &protect)?;
            println!(
                "Created repository {name} (protected: {})",
                protect.join(", ")
            );
        }
        Command::Token(TokenCommand::Create {
            name,
            agent,
            on_behalf_of,
            allow,
            admin,
        }) => {
            let allow = if allow.is_empty() {
                if agent {
                    vec![format!("agents/{name}/*")]
                } else {
                    vec!["*".to_owned()]
                }
            } else {
                allow
            };
            let info = TokenInfo {
                name: name.clone(),
                kind: if agent { "agent" } else { "human" }.to_owned(),
                on_behalf_of,
                allow: allow.clone(),
                admin,
            };
            let token = server.create_token(&info)?;
            println!("{token}");
            eprintln!(
                "Token for {name} ({}) may move: {}. Store it now; it is not shown again.",
                info.kind,
                allow.join(", ")
            );
        }
    }
    Ok(())
}
