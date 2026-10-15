//! `hebra-server`: run the server, and administer it (people, organizations,
//! repositories, mandates, keys).
//!
//! Every option can also be set with an environment variable (shown in
//! `--help`), which is how the container image and the Helm chart configure it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use hebra_core::store::Key;
use hebra_server::db::{AccountKind, OrgRole};
use hebra_server::identity::secret;
use hebra_server::{
    Config, Edition, GithubOauth, Limits, ModelGateway, NatsConfig, Server, Signup, Stripe,
};

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
    /// Key that signs mandates (`hebra-server keygen mandate`). Required
    /// when replicas share a database.
    #[arg(long, global = true, env = "HEBRA_MANDATE_KEY", hide_env_values = true)]
    mandate_key: Option<String>,
    /// Memory for cached objects, in MiB.
    #[arg(long, global = true, env = "HEBRA_CACHE_MIB", default_value_t = 256)]
    cache_mib: usize,
    /// Which product this is.
    #[arg(long, global = true, env = "HEBRA_EDITION", value_enum, default_value_t = EditionArg::Community)]
    edition: EditionArg,
    /// Where people reach this server (used in sign-in links).
    #[arg(
        long,
        global = true,
        env = "HEBRA_PUBLIC_URL",
        default_value = "http://127.0.0.1:4600"
    )]
    public_url: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum EditionArg {
    Community,
    Cloud,
}

#[derive(Clone, Copy, ValueEnum)]
enum SignupArg {
    Open,
    Invite,
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
    /// Who may create an account: anyone signing in, or invited people only.
    #[arg(long, env = "HEBRA_SIGNUP", value_enum, default_value_t = SignupArg::Invite)]
    signup: SignupArg,
    /// GitHub OAuth app for "Sign in with GitHub".
    #[arg(
        long,
        env = "HEBRA_GITHUB_CLIENT_ID",
        requires = "github_client_secret"
    )]
    github_client_id: Option<String>,
    #[arg(
        long,
        env = "HEBRA_GITHUB_CLIENT_SECRET",
        hide_env_values = true,
        requires = "github_client_id"
    )]
    github_client_secret: Option<String>,
    /// Plans file (Cloud); the built-in plans when absent.
    #[arg(long, env = "HEBRA_BILLING_PLANS")]
    plans: Option<PathBuf>,
    /// Stripe secret key and webhook signing secret (Cloud).
    #[arg(
        long,
        env = "HEBRA_STRIPE_SECRET_KEY",
        hide_env_values = true,
        requires = "stripe_webhook_secret"
    )]
    stripe_secret_key: Option<String>,
    #[arg(
        long,
        env = "HEBRA_STRIPE_WEBHOOK_SECRET",
        hide_env_values = true,
        requires = "stripe_secret_key"
    )]
    stripe_webhook_secret: Option<String>,
    /// Anthropic API key for agent verification (Cloud).
    #[arg(long, env = "HEBRA_ANTHROPIC_API_KEY", hide_env_values = true)]
    anthropic_api_key: Option<String>,
    /// The model agent verification uses.
    #[arg(
        long,
        env = "HEBRA_VERIFIER_MODEL",
        default_value = "claude-sonnet-5-5"
    )]
    verifier_model: String,
    #[arg(
        long,
        env = "HEBRA_ANTHROPIC_BASE_URL",
        default_value = "https://api.anthropic.com"
    )]
    anthropic_base_url: String,
    /// Log as JSON lines (for log collectors).
    #[arg(long, env = "HEBRA_LOG_JSON")]
    log_json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP(S) server.
    Serve(Box<ServeArgs>),
    /// Print a new random key.
    Keygen {
        #[arg(value_enum, default_value_t = KeyKind::Encryption)]
        kind: KeyKind,
    },
    /// People.
    #[command(subcommand)]
    User(UserCommand),
    /// Organizations.
    #[command(subcommand)]
    Org(OrgCommand),
    /// Repositories.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Mandates (credentials for people, agents and automation).
    #[command(subcommand)]
    Mandate(MandateCommand),
}

#[derive(Clone, Copy, ValueEnum)]
enum KeyKind {
    /// For HEBRA_ENCRYPTION_KEYS.
    Encryption,
    /// For HEBRA_MANDATE_KEY.
    Mandate,
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create a person and print a one-time sign-in link.
    Create {
        handle: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        email: Option<String>,
        /// Administers the whole server.
        #[arg(long)]
        admin: bool,
    },
    /// Print a new one-time sign-in link for an existing person.
    Invite { handle: String },
}

#[derive(Subcommand)]
enum OrgCommand {
    /// Create an organization owned by a person.
    Create {
        handle: String,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum RepoCommand {
    /// Create a repository `owner/name`.
    Create {
        full_name: String,
        #[arg(long)]
        public: bool,
        /// Bookmarks only people can move (agents push elsewhere).
        #[arg(long = "protect", default_values_t = vec!["main".to_owned()])]
        protect: Vec<String>,
    },
}

#[derive(Subcommand)]
enum MandateCommand {
    /// Issue a mandate for an account (prints it once).
    Issue {
        handle: String,
        #[arg(long, default_value = "issued by an administrator")]
        label: String,
        /// Lifetime: `90d`, `12h`, `30m`.
        #[arg(long, value_parser = parse_duration)]
        ttl: Option<i64>,
    },
    /// Revoke a mandate by id (and everything derived from it).
    Revoke { handle: String, id: String },
}

fn parse_duration(text: &str) -> Result<i64, String> {
    let (n, unit) = text.split_at(text.len().saturating_sub(1));
    let n: i64 = n
        .parse()
        .map_err(|_| format!("invalid duration `{text}`"))?;
    let secs = match unit {
        "d" => n * 86_400,
        "h" => n * 3_600,
        "m" => n * 60,
        "s" => n,
        _ => return Err(format!("invalid duration `{text}` (use d, h, m or s)")),
    };
    Ok(secs)
}

impl Storage {
    fn config(&self) -> anyhow::Result<Config> {
        let mut config = Config::local(&self.data);
        config.edition = match self.edition {
            EditionArg::Community => Edition::Community,
            EditionArg::Cloud => Edition::Cloud,
        };
        config.public_url = self.public_url.trim_end_matches('/').to_owned();
        config.database_url = self.database_url.clone();
        config.db_max_connections = self.database_max_connections;
        config.cache_bytes = self.cache_mib * 1024 * 1024;
        config.mandate_key = self.mandate_key.clone();
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

async fn serve(mut config: Config, args: ServeArgs) -> anyhow::Result<()> {
    init_tracing(args.log_json);
    config.nats = args.live.nats_url.map(|url| NatsConfig {
        url,
        creds: args.live.nats_creds,
        ca: args.live.nats_ca,
        cert: args.live.nats_cert,
        key: args.live.nats_key,
    });
    config.trust_proxy = args.trust_proxy;
    config.hsts = args.hsts || args.tls_cert.is_some();
    config.cors_origins = args.cors_origins;
    config.limits = Limits {
        token_rps: args.rate_token,
        token_burst: args.burst_token,
        ip_rps: args.rate_ip,
        ip_burst: args.burst_ip,
    };
    config.signup = match args.signup {
        SignupArg::Open => Signup::Open,
        SignupArg::Invite => Signup::Invite,
    };
    config.github =
        args.github_client_id
            .zip(args.github_client_secret)
            .map(|(client_id, client_secret)| GithubOauth {
                client_id,
                client_secret,
            });
    config.plans = args.plans;
    config.stripe = args.stripe_secret_key.zip(args.stripe_webhook_secret).map(
        |(secret_key, webhook_secret)| Stripe {
            secret_key,
            webhook_secret,
        },
    );
    config.gateway = args.anthropic_api_key.map(|api_key| ModelGateway {
        api_key,
        model: args.verifier_model,
        base_url: args.anthropic_base_url,
    });
    if config.encryption_keys.is_empty() {
        tracing::warn!(
            "objects are stored unencrypted; set HEBRA_ENCRYPTION_KEYS (see `hebra-server keygen`)"
        );
    }
    let server = Arc::new(Server::open(config).await?);
    server.spawn_maintenance();
    if let Some(metrics_addr) = args.metrics_addr {
        let listener = tokio::net::TcpListener::bind(metrics_addr).await?;
        let app = hebra_server::metrics_router(Arc::clone(&server));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        tracing::info!(%metrics_addr, "metrics listening");
    }
    let addr = args.addr;
    let scheme = if args.tls_cert.is_some() {
        "https"
    } else {
        "http"
    };
    tracing::info!(
        "hebra-server {} listening on {scheme}://{addr} ({})",
        env!("CARGO_PKG_VERSION"),
        server.describe()
    );
    match (args.tls_cert, args.tls_key) {
        (Some(cert), Some(key)) => {
            hebra_server::serve_tls_until(server, addr, cert, key, hebra_server::shutdown_signal())
                .await?;
        }
        _ => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            hebra_server::serve_until(server, listener, hebra_server::shutdown_signal()).await?;
        }
    }
    tracing::info!("stopped");
    Ok(())
}

fn api<T>(r: Result<T, hebra_server::ApiError>) -> anyhow::Result<T> {
    r.map_err(|e| anyhow::anyhow!(e.message().to_owned()))
}

/// A one-time sign-in link for `account_id`, valid for a week.
async fn invitation(server: &Server, account_id: &str, public_url: &str) -> anyhow::Result<String> {
    let code = secret::token(secret::INVITE_PREFIX);
    api(server
        .db()
        .create_invitation(&secret::hash(&code), account_id, 7 * 24 * 3600 * 1000)
        .await)?;
    Ok(format!("{public_url}/login/invite?code={code}"))
}

async fn account(server: &Server, handle: &str) -> anyhow::Result<hebra_server::db::Account> {
    api(server.db().account_by_handle(handle).await)?
        .ok_or_else(|| anyhow::anyhow!("`{handle}` not found"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // One crypto provider for every TLS connection (HTTPS, Postgres, NATS).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    let config = cli.storage.config()?;
    let public_url = config.public_url.clone();
    match cli.command {
        Command::Serve(args) => serve(config, *args).await?,
        Command::Keygen { kind } => match kind {
            KeyKind::Encryption => println!("{}", Key::generate_hex()),
            KeyKind::Mandate => println!("{}", hebra_mandate::Authority::generate().to_hex()),
        },
        Command::User(cmd) => {
            let server = Server::open(config).await?;
            let db = server.db();
            match cmd {
                UserCommand::Create {
                    handle,
                    name,
                    email,
                    admin,
                } => {
                    anyhow::ensure!(
                        hebra_server::names::valid_handle(&handle),
                        "a handle has 1–39 lowercase letters, digits or hyphens"
                    );
                    let created = api(db
                        .create_account(
                            &handle,
                            AccountKind::Human,
                            name.as_deref().unwrap_or(&handle),
                            email.as_deref(),
                            None,
                        )
                        .await)?;
                    if admin || api(db.count_humans().await)? == 1 {
                        api(db.set_site_admin(&created.id, true).await)?;
                    }
                    println!("{}", invitation(&server, &created.id, &public_url).await?);
                    eprintln!(
                        "Created {handle}. Send them this one-time sign-in link (valid 7 days)."
                    );
                }
                UserCommand::Invite { handle } => {
                    let person = account(&server, &handle).await?;
                    println!("{}", invitation(&server, &person.id, &public_url).await?);
                }
            }
        }
        Command::Org(OrgCommand::Create {
            handle,
            owner,
            name,
        }) => {
            let server = Server::open(config).await?;
            let db = server.db();
            anyhow::ensure!(
                hebra_server::names::valid_handle(&handle),
                "invalid organization name"
            );
            let person = account(&server, &owner).await?;
            let org = api(db
                .create_account(
                    &handle,
                    AccountKind::Org,
                    name.as_deref().unwrap_or(&handle),
                    None,
                    None,
                )
                .await)?;
            api(db.set_member(&org.id, &person.id, OrgRole::Owner).await)?;
            println!("Created organization {handle} (owner: {owner})");
        }
        Command::Repo(RepoCommand::Create {
            full_name,
            public,
            protect,
        }) => {
            let server = Server::open(config).await?;
            let (owner, name) = full_name
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("use owner/name"))?;
            anyhow::ensure!(
                hebra_server::names::valid_repo_name(name),
                "invalid repository name"
            );
            let owner = account(&server, owner).await?;
            let repo = api(server
                .db()
                .create_repo(
                    &owner.handle,
                    &hebra_server::db::NewRepo {
                        owner_id: &owner.id,
                        name,
                        public,
                        description: "",
                        protect: &protect,
                        forked_from: None,
                    },
                )
                .await)?;
            server.create_objects(&repo.id)?;
            println!("Created {full_name} (protected: {})", protect.join(", "));
        }
        Command::Mandate(cmd) => {
            let server = Server::open(config).await?;
            match cmd {
                MandateCommand::Issue { handle, label, ttl } => {
                    let holder = account(&server, &handle).await?;
                    let expires = ttl.map(|t| jiff::Timestamp::now().as_second() + t);
                    let (mandate, id) = server.authority().issue(&holder.id, expires)?;
                    api(server
                        .db()
                        .record_mandate(
                            &id,
                            &holder.id,
                            &label,
                            Some("admin"),
                            expires.map(|e| e * 1000),
                        )
                        .await)?;
                    println!("{mandate}");
                    eprintln!("Mandate {id} for {handle}. Store it now; it is not shown again.");
                }
                MandateCommand::Revoke { handle, id } => {
                    let holder = account(&server, &handle).await?;
                    anyhow::ensure!(
                        api(server
                            .db()
                            .revoke_mandate(&id, &holder.id, "revoked by an administrator")
                            .await)?,
                        "no such mandate"
                    );
                    println!("Revoked {id}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90d").unwrap(), 90 * 86_400);
        assert_eq!(parse_duration("12h").unwrap(), 12 * 3_600);
        assert!(parse_duration("12").is_err());
        assert!(parse_duration("xd").is_err());
    }
}
