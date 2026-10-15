//! How a server is put together: edition, storage, events, identity,
//! limits, billing and the model gateway.

use std::path::PathBuf;

use hebra_core::store::Key;

use crate::bus::NatsConfig;
use crate::objects::ObjectsConfig;
use crate::ratelimit::Limits;

/// Which product this server is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Edition {
    /// Self-hosted, free: everything except metered and managed features.
    Community,
    /// hebra.dev: plans, billing, agent verification, merge queue.
    Cloud,
}

/// Who may create an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Signup {
    /// Anyone who signs in with a configured identity provider.
    Open,
    /// Only people a site admin invited.
    Invite,
}

#[derive(Debug, Clone)]
pub struct GithubOauth {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone)]
pub struct Stripe {
    pub secret_key: String,
    pub webhook_secret: String,
}

#[derive(Debug, Clone)]
pub struct ModelGateway {
    /// Anthropic API key, used only by the server.
    pub api_key: String,
    /// The model every verification uses.
    pub model: String,
    pub base_url: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub edition: Edition,
    /// Where people reach this server (`https://hebra.dev`); used in sign-in
    /// links and device verification.
    pub public_url: String,
    /// Local state: the SQLite database and, unless S3 is used, objects.
    pub data: PathBuf,
    /// `postgres://…` to share metadata between replicas; SQLite otherwise.
    pub database_url: Option<String>,
    pub db_max_connections: u32,
    pub objects: ObjectsConfig,
    /// Memory for cached objects, per replica.
    pub cache_bytes: usize,
    /// Object encryption keys; the first one encrypts.
    pub encryption_keys: Vec<Key>,
    /// Key that signs mandates (64 hex). Required with several replicas;
    /// a single server keeps one in `<data>/mandate.key`.
    pub mandate_key: Option<String>,
    pub nats: Option<NatsConfig>,
    pub limits: Limits,
    /// Behind an ingress: take the client IP from `X-Real-IP`.
    pub trust_proxy: bool,
    /// Send `Strict-Transport-Security` (HTTPS here or at the ingress).
    pub hsts: bool,
    /// Browser origins allowed to call the API directly (none by default;
    /// the web app calls it from its server).
    pub cors_origins: Vec<String>,
    pub signup: Signup,
    pub github: Option<GithubOauth>,
    /// Plans file (Cloud).
    pub plans: Option<PathBuf>,
    pub stripe: Option<Stripe>,
    pub gateway: Option<ModelGateway>,
}

impl Config {
    /// A single Community server with everything on local disk: SQLite,
    /// objects in `data`, no NATS, invitations only.
    #[must_use]
    pub fn local(data: &std::path::Path) -> Self {
        Self {
            edition: Edition::Community,
            public_url: "http://127.0.0.1:4600".to_owned(),
            data: data.to_path_buf(),
            database_url: None,
            db_max_connections: 16,
            objects: ObjectsConfig::Local(data.join("repos")),
            cache_bytes: 256 * 1024 * 1024,
            encryption_keys: Vec::new(),
            mandate_key: None,
            nats: None,
            limits: Limits::default(),
            trust_proxy: false,
            hsts: false,
            cors_origins: Vec::new(),
            signup: Signup::Invite,
            github: None,
            plans: None,
            stripe: None,
            gateway: None,
        }
    }
}
