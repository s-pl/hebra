//! How a server is put together: storage, events, limits.

use std::path::PathBuf;

use hebra_core::store::Key;

use crate::bus::NatsConfig;
use crate::objects::ObjectsConfig;
use crate::ratelimit::Limits;

#[derive(Debug, Clone)]
pub struct Config {
    /// Local state: the SQLite database and, unless S3 is used, objects.
    pub data: PathBuf,
    /// `postgres://…` to share metadata between replicas.
    pub database_url: Option<String>,
    pub db_max_connections: u32,
    pub objects: ObjectsConfig,
    /// Memory for cached objects, per replica.
    pub cache_bytes: usize,
    /// Object encryption keys; the first one encrypts.
    pub encryption_keys: Vec<Key>,
    pub nats: Option<NatsConfig>,
    pub limits: Limits,
    /// Behind an ingress: take the client IP from `X-Real-IP`.
    pub trust_proxy: bool,
    /// Send `Strict-Transport-Security` (HTTPS here or at the ingress).
    pub hsts: bool,
    /// Browser origins allowed to call the API directly (none by default;
    /// the web app calls it from its server).
    pub cors_origins: Vec<String>,
}

impl Config {
    /// Everything on local disk: SQLite, objects in `data`, no NATS.
    #[must_use]
    pub fn local(data: &std::path::Path) -> Self {
        Self {
            data: data.to_path_buf(),
            database_url: None,
            db_max_connections: 20,
            objects: ObjectsConfig::Local(data.join("repos")),
            cache_bytes: 256 * 1024 * 1024,
            encryption_keys: Vec::new(),
            nats: None,
            limits: Limits::default(),
            trust_proxy: false,
            hsts: false,
            cors_origins: Vec::new(),
        }
    }
}
