//! `hebra-server`: run the server and manage repositories and tokens.
//!
//! Every option can also be set with an environment variable (shown in
//! `--help`), which is how the container image and the Helm chart configure it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use hebra_core::store::Key;
use hebra_server::{Config, Limits, NatsConfig, Server, TokenInfo};

#[derive(Parser)]
#[command(
    name = "hebra-server",
    version,
    about = "Host Hebra repositories for people and agents"
)]
struct Cli {
    #[command(flatten)]
    storage: Storage,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Storage {
    /// Directory for local state (SQLite database, objects unless S3 is used).
    #[arg(long, global = true, env = "HEBRA_DATA", default_value = "hebra-data")]
    data: PathBuf,
    /// PostgreSQL URL for metadata shared by many replicas
    /// (e.g. `postgres://hebra@db/hebra?sslmode=verify-full`).
    #[arg(
        long,
        global = true,
        env = "HEBRA_DATABASE_URL",
        hide_env_values = true
    )]
    database_url: Option<String>,
    #[arg(
        long,
        global = true,
        env = "HEBRA_DATABASE_MAX_CONNECTIONS",
        default_value_t = 20
    )]
    database_max_connections: u32,
    /// Keep objects in this S3 bucket (credentials, region and endpoint from
    /// the standard AWS_* variables or the pod's IAM role).
    #[cfg(feature = "s3")]
    #[arg(long, global = true, env = "HEBRA_S3_BUCKET")]
    s3_bucket: Option<String>,
    /// Object encryption keys, 64 hex characters each, comma separated. The
    /// first encrypts new objects; all of them can decrypt (key rotation).
    #[arg(
        long,
        global = true,
        env = "HEBRA_ENCRYPTION_KEYS",
        hide_env_values = true,
        value_delimiter = ','
    )]
    encryption_keys: Vec<String>,
    /// Memory for cached objects, in MiB.
    #[arg(long, global = true, env = "HEBRA_CACHE_MIB", default_value_t = 256)]
    cache_mib: usize,
}

#[derive(Args)]
struct Live {
    /// NATS servers for live events between replicas (`nats://` or `tls://`).
    #[arg(long, env = "HEBRA_NATS_URL")]
    nats_url: Option<String>,
    /// NATS credentials file (JWT + NKey).
    #[arg(long, env = "HEBRA_NATS_CREDS")]
    nats_creds: Option<PathBuf>,
    /// CA bundle that signed the NATS servers' certificates.
    #[arg(long, env = "HEBRA_NATS_CA")]
    nats_ca: Option<PathBuf>,
    /// Client certificate and key for mutual TLS with NATS.
    #[arg(long, env = "HEBRA_NATS_CERT", requires = "nats_key")]
    nats_cert: Option<PathBuf>,
    #[arg(long, env = "HEBRA_NATS_KEY", requires = "nats_cert")]
    nats_key: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP(S) server.
    Serve(Box<ServeArgs>),
    /// Print a new random encryption key for HEBRA_ENCRYPTION_KEYS.
    Keygen,
    /// Manage repositories.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Manage access tokens.
    #[command(subcommand)]
    Token(TokenCommand),
}

#[derive(Args)]
struct ServeArgs {
    #[arg(long, env = "HEBRA_ADDR", default_value = "127.0.0.1:4600")]
    addr: SocketAddr,
    /// Serve HTTPS with this certificate chain (PEM) …
    #[arg(long, env = "HEBRA_TLS_CERT", requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// … and this private key (PEM).
    #[arg(long, env = "HEBRA_TLS_KEY", requires = "tls_cert")]
    tls_key: Option<PathBuf>,
    /// Serve Prometheus metrics on this (internal) address.
    #[arg(long, env = "HEBRA_METRICS_ADDR")]
    metrics_addr: Option<SocketAddr>,
    #[command(flatten)]
    live: Live,
    /// Running behind a reverse proxy: take client IPs from X-Real-IP.
    #[arg(long, env = "HEBRA_TRUST_PROXY")]
    trust_proxy: bool,
    /// Send HSTS (set when HTTPS terminates at the ingress).
    #[arg(long, env = "HEBRA_HSTS")]
    hsts: bool,
    /// Browser origins allowed to call the API directly (comma separated).
    #[arg(long, env = "HEBRA_CORS_ORIGINS", value_delimiter = ',')]
    cors_origins: Vec<String>,
    /// Requests per second per token, and burst.
    #[arg(long, env = "HEBRA_RATE_TOKEN", default_value_t = 100.0)]
    rate_token: f64,
    #[arg(long, env = "HEBRA_BURST_TOKEN", default_value_t = 1000.0)]
    burst_token: f64,
    /// Requests per second per client IP, and burst.
    #[arg(long, env = "HEBRA_RATE_IP", default_value_t = 200.0)]
    rate_ip: f64,
    #[arg(long, env = "HEBRA_BURST_IP", default_value_t = 2000.0)]
    burst_ip: f64,
    /// Log as JSON lines (for log collectors).
    #[arg(long, env = "HEBRA_LOG_JSON")]
    log_json: bool,
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
        /// Stop working after this long: `90d`, `12h`, `30m`. Agent tokens
        /// default to 90 days; tokens for people do not expire unless set.
        #[arg(long, value_parser = parse_duration)]
        expires: Option<Duration>,
        /// Never expire, even for agents.
        #[arg(long, conflicts_with = "expires")]
        no_expiry: bool,
    },
    /// List tokens (names and permissions; secrets are never stored).
    List,
    /// Revoke every token with this name.
    Revoke {
        #[arg(long)]
        name: String,
    },
}

fn parse_duration(text: &str) -> Result<Duration, String> {
    let (n, unit) = text.split_at(text.len().saturating_sub(1));
    let n: u64 = n
        .parse()
        .map_err(|_| format!("invalid duration `{text}`"))?;
    let secs = match unit {
        "d" => n * 86_400,
        "h" => n * 3_600,
        "m" => n * 60,
        "s" => n,
        _ => return Err(format!("invalid duration `{text}` (use d, h, m or s)")),
    };
    Ok(Duration::from_secs(secs))
}

const AGENT_TOKEN_TTL: Duration = Duration::from_secs(90 * 86_400);

impl Storage {
    fn config(&self) -> anyhow::Result<Config> {
        let mut config = Config::local(&self.data);
        config.database_url = self.database_url.clone();
        config.db_max_connections = self.database_max_connections;
        config.cache_bytes = self.cache_mib * 1024 * 1024;
        #[cfg(feature = "s3")]
        if let Some(bucket) = &self.s3_bucket {
            config.objects = hebra_server::ObjectsConfig::S3 {
                bucket: bucket.clone(),
            };
        }
        config.encryption_keys = self
            .encryption_keys
            .iter()
            .filter(|k| !k.trim().is_empty())
            .map(|k| Key::from_hex(k))
            .collect::<Result<_, _>>()?;
        Ok(config)
    }
}

fn init_tracing(json: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("HEBRA_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // One crypto provider for every TLS connection (HTTPS, Postgres, NATS).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    let mut config = cli.storage.config()?;
    match cli.command {
        Command::Keygen => {
            println!("{}", Key::generate_hex());
            return Ok(());
        }
        Command::Serve(args) => {
            let ServeArgs {
                addr,
                tls_cert,
                tls_key,
                metrics_addr,
                live,
                trust_proxy,
                hsts,
                cors_origins,
                rate_token,
                burst_token,
                rate_ip,
                burst_ip,
                log_json,
            } = *args;
            init_tracing(log_json);
            config.nats = live.nats_url.map(|url| NatsConfig {
                url,
                creds: live.nats_creds,
                ca: live.nats_ca,
                cert: live.nats_cert,
                key: live.nats_key,
            });
            config.trust_proxy = trust_proxy;
            config.hsts = hsts || tls_cert.is_some();
            config.cors_origins = cors_origins;
            config.limits = Limits {
                token_rps: rate_token,
                token_burst: burst_token,
                ip_rps: rate_ip,
                ip_burst: burst_ip,
            };
            if config.encryption_keys.is_empty() {
                tracing::warn!(
                    "objects are stored unencrypted; set HEBRA_ENCRYPTION_KEYS (see `hebra-server keygen`)"
                );
            }
            let server = Arc::new(Server::open(config).await?);
            server.spawn_maintenance();
            if let Some(metrics_addr) = metrics_addr {
                let listener = tokio::net::TcpListener::bind(metrics_addr).await?;
                let app = hebra_server::metrics_router(Arc::clone(&server));
                tokio::spawn(async move {
                    let _ = axum::serve(listener, app).await;
                });
                tracing::info!(%metrics_addr, "metrics listening");
            }
            let scheme = if tls_cert.is_some() { "https" } else { "http" };
            tracing::info!(
                "hebra-server {} listening on {scheme}://{addr} ({})",
                env!("CARGO_PKG_VERSION"),
                server.describe()
            );
            match (tls_cert, tls_key) {
                (Some(cert), Some(key)) => {
                    hebra_server::serve_tls_until(
                        server,
                        addr,
                        cert,
                        key,
                        hebra_server::shutdown_signal(),
                    )
                    .await?;
                }
                _ => {
                    let listener = tokio::net::TcpListener::bind(addr).await?;
                    hebra_server::serve_until(server, listener, hebra_server::shutdown_signal())
                        .await?;
                }
            }
            tracing::info!("stopped");
        }
        Command::Repo(RepoCommand::Create { name, protect }) => {
            let server = Server::open(config).await?;
            server.create_repo(&name, &protect).await?;
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
            expires,
            no_expiry,
        }) => {
            let server = Server::open(config).await?;
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
            let ttl = match (expires, no_expiry, agent) {
                (Some(t), _, _) => Some(t),
                (None, false, true) => Some(AGENT_TOKEN_TTL),
                _ => None,
            };
            let token = server.create_token(&info, ttl).await?;
            println!("{token}");
            let expiry = ttl.map_or_else(
                || "never expires".to_owned(),
                |t| format!("expires in {} days", t.as_secs().div_ceil(86_400)),
            );
            eprintln!(
                "Token for {name} ({}) may move: {}; {expiry}. Store it now; it is not shown again.",
                info.kind,
                allow.join(", ")
            );
        }
        Command::Token(TokenCommand::List) => {
            let server = Server::open(config).await?;
            for t in server.list_tokens().await? {
                let expiry = t.expires_at.map_or_else(
                    || "no expiry".to_owned(),
                    |at| {
                        jiff::Timestamp::from_second(at)
                            .map_or_else(|_| at.to_string(), |ts| format!("expires {ts}"))
                    },
                );
                println!(
                    "{}\t{}{}\t{}\t{}\t{expiry}",
                    t.name,
                    t.kind,
                    if t.admin { " (admin)" } else { "" },
                    t.allow.join(", "),
                    t.created_at
                );
            }
        }
        Command::Token(TokenCommand::Revoke { name }) => {
            let server = Server::open(config).await?;
            let n = server.revoke_tokens(&name).await?;
            println!("Revoked {n} token(s) named {name}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(
            parse_duration("90d").unwrap(),
            Duration::from_secs(90 * 86_400)
        );
        assert_eq!(
            parse_duration("12h").unwrap(),
            Duration::from_secs(12 * 3_600)
        );
        assert!(parse_duration("12").is_err());
        assert!(parse_duration("xd").is_err());
    }
}
