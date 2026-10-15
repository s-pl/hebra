//! The state every request shares: the database, objects, the event bus,
//! the mandate authority, and the per-replica caches in front of them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::StatusCode;
use hebra_core::ObjectId;
use hebra_core::store::Store;
use hebra_mandate::Authority;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::api::blocking;
use crate::billing::Plans;
use crate::bus::{Bus, LiveEvent};
use crate::cache::TtlCache;
use crate::config::{Config, Edition, GithubOauth, ModelGateway, Signup, Stripe};
use crate::db::{self, Db, Repo, RepoRole};
use crate::error::{ApiError, ApiResult};
use crate::identity::{AUTH_TTL, Principal};
use crate::metrics::Metrics;
use crate::objects::Objects;
use crate::ratelimit::RateLimiter;

/// Presence entries older than this are hidden (and later deleted).
pub(crate) const PRESENCE_TTL_MS: i64 = 15 * 60 * 1000;
/// Upper bound for a cached bookmark map if an invalidation is ever lost.
const BOOKMARKS_TTL: Duration = Duration::from_secs(30);

/// Per-replica caches. Entries are short-lived or invalidated by events, so
/// replicas never need to coordinate.
pub(crate) struct Caches {
    /// Credential hash -> caller (`None`: unknown or revoked).
    pub principals: TtlCache<Option<Arc<Principal>>>,
    /// `owner/name` -> repository (`None`: does not exist).
    pub repos: TtlCache<Option<Arc<Repo>>>,
    /// `account:repo` -> role.
    pub roles: TtlCache<Option<RepoRole>>,
    /// Repository id -> bookmarks; dropped when a bookmark moves anywhere.
    pub bookmarks: Arc<TtlCache<Arc<BTreeMap<String, String>>>>,
    /// Repository id -> protected patterns.
    pub protected: TtlCache<Arc<Vec<String>>>,
}

pub struct Server {
    pub(crate) edition: Edition,
    pub(crate) public_url: String,
    pub(crate) signup: Signup,
    pub(crate) github: Option<GithubOauth>,
    pub(crate) stripe: Option<Stripe>,
    pub(crate) gateway: Option<ModelGateway>,
    pub(crate) plans: Option<Plans>,
    pub(crate) db: Db,
    pub(crate) objects: Objects,
    pub(crate) bus: Bus,
    pub(crate) authority: Authority,
    pub(crate) http: reqwest::Client,
    pub(crate) limiter: RateLimiter,
    pub(crate) metrics: Metrics,
    pub(crate) hsts: bool,
    pub(crate) cors_origins: Vec<String>,
    pub(crate) caches: Caches,
    pub(crate) shutdown: watch::Sender<bool>,
}

/// The mandate signing key: configured, or kept in the data directory of a
/// single server.
fn authority(config: &Config) -> anyhow::Result<Authority> {
    if let Some(hex) = &config.mandate_key {
        return Ok(Authority::from_hex(hex)?);
    }
    anyhow::ensure!(
        config.database_url.is_none(),
        "HEBRA_MANDATE_KEY is required when replicas share a database (generate one with `hebra-server keygen --mandate`)"
    );
    let path = config.data.join("mandate.key");
    if let Ok(hex) = std::fs::read_to_string(&path) {
        return Ok(Authority::from_hex(&hex)?);
    }
    let fresh = Authority::generate();
    crate::objects::write_private(&path, fresh.to_hex().as_bytes())?;
    Ok(fresh)
}

impl Server {
    pub async fn open(config: Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data)?;
        let db = match &config.database_url {
            Some(url) => Db::connect(url, config.db_max_connections).await?,
            None => Db::sqlite_in(&config.data).await?,
        };
        let objects = Objects::open(
            &config.objects,
            config.encryption_keys.clone(),
            config.cache_bytes,
        )?;
        let bus = match &config.nats {
            Some(nats) => Bus::nats(nats).await?,
            None => Bus::local(),
        };
        let plans = match (config.edition, &config.plans) {
            (Edition::Cloud, Some(path)) => Some(Plans::load(path)?),
            (Edition::Cloud, None) => Some(Plans::builtin()),
            (Edition::Community, _) => None,
        };
        let caches = Caches {
            principals: TtlCache::new(100_000),
            repos: TtlCache::new(100_000),
            roles: TtlCache::new(200_000),
            bookmarks: Arc::new(TtlCache::new(20_000)),
            protected: TtlCache::new(100_000),
        };
        spawn_invalidation(&bus, Arc::clone(&caches.bookmarks));
        Ok(Self {
            edition: config.edition,
            public_url: config.public_url.trim_end_matches('/').to_owned(),
            signup: config.signup,
            github: config.github.clone(),
            stripe: config.stripe.clone(),
            gateway: config.gateway.clone(),
            plans,
            authority: authority(&config)?,
            db,
            objects,
            bus,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("hebra-server/", env!("CARGO_PKG_VERSION")))
                .build()?,
            limiter: RateLimiter::new(config.limits, config.trust_proxy),
            metrics: Metrics::default(),
            hsts: config.hsts,
            cors_origins: config.cors_origins,
            caches,
            shutdown: watch::channel(false).0,
        })
    }

    /// One line describing the setup, for startup logs.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "edition: {}, metadata: {}, events: {}, objects encrypted: {}",
            match self.edition {
                Edition::Community => "community",
                Edition::Cloud => "cloud",
            },
            self.db.backend(),
            self.bus.backend(),
            if self.objects.encrypted() {
                "yes"
            } else {
                "no"
            }
        )
    }

    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }

    #[must_use]
    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    /// Prepares storage for a new repository.
    pub fn create_objects(&self, repo_id: &str) -> std::io::Result<()> {
        self.objects.create(repo_id)
    }

    /// A repository's objects (and, for forks, those of its origins).
    pub(crate) fn store(&self, repo: &Repo) -> Store {
        if repo.lineage.is_empty() {
            return self.objects.store(&repo.id);
        }
        let chain: Vec<String> = std::iter::once(repo.id.clone())
            .chain(repo.lineage.iter().cloned())
            .collect();
        self.objects.store_chain(&chain)
    }

    /// Tells live subscribers of a repository, on every replica.
    pub(crate) fn notify(&self, repo: &Repo, kind: &str, data: Value) {
        self.bus.publish(LiveEvent {
            repo: repo.id.clone(),
            kind: kind.to_owned(),
            data,
        });
    }

    /// Forgets what this replica cached about a repository (after a change
    /// of settings or access).
    pub(crate) fn forget_repo(&self, repo: &Repo) {
        self.caches.repos.remove(&repo.full_name);
        self.caches.protected.remove(&repo.id);
        self.caches.bookmarks.remove(&repo.id);
        self.caches.roles.clear();
    }

    /// All bookmarks of a repository, for reads (may lag a write on another
    /// replica by the time its event takes to arrive).
    pub(crate) async fn bookmarks(&self, repo: &Repo) -> ApiResult<Arc<BTreeMap<String, String>>> {
        if let Some(cached) = self.caches.bookmarks.get(&repo.id) {
            return Ok(cached);
        }
        let fresh = Arc::new(self.db.bookmarks(&repo.id).await?);
        self.caches
            .bookmarks
            .put(&repo.id, Arc::clone(&fresh), BOOKMARKS_TTL);
        Ok(fresh)
    }

    pub(crate) async fn bookmark(&self, repo: &Repo, name: &str) -> ApiResult<Option<ObjectId>> {
        self.bookmarks(repo)
            .await?
            .get(name)
            .map(|t| ObjectId::parse(t).map_err(ApiError::from))
            .transpose()
    }

    /// A bookmark straight from the database, for decisions that write.
    pub(crate) async fn bookmark_fresh(
        &self,
        repo: &Repo,
        name: &str,
    ) -> ApiResult<Option<ObjectId>> {
        self.db
            .bookmark(&repo.id, name)
            .await?
            .map(|t| ObjectId::parse(&t).map_err(ApiError::from))
            .transpose()
    }

    pub(crate) async fn protected(&self, repo_id: &str) -> ApiResult<Arc<Vec<String>>> {
        if let Some(cached) = self.caches.protected.get(repo_id) {
            return Ok(cached);
        }
        let fresh = Arc::new(self.db.protected(repo_id).await?);
        self.caches
            .protected
            .put(repo_id, Arc::clone(&fresh), AUTH_TTL);
        Ok(fresh)
    }

    /// A bookmark name or a commit id, to a commit that exists.
    pub(crate) async fn resolve(&self, repo: &Repo, rev: &str) -> ApiResult<ObjectId> {
        if let Some(target) = self.bookmark(repo, rev).await? {
            return Ok(target);
        }
        let id = ObjectId::parse(rev)
            .map_err(|_| ApiError::not_found(format!("revision `{rev}` not found")))?;
        let store = self.store(repo);
        if blocking(move || Ok(store.has(&id))).await? {
            Ok(id)
        } else {
            Err(ApiError::not_found(format!("revision `{rev}` not found")))
        }
    }

    /// Compare-and-swap a bookmark (atomic across replicas), record the event
    /// and tell live clients. Permission is the caller's job.
    pub(crate) async fn move_bookmark(
        &self,
        repo: &Repo,
        who: &Principal,
        name: &str,
        old: Option<String>,
        new: Option<String>,
    ) -> ApiResult<()> {
        let actor = who.actor();
        let result = self
            .db
            .cas_bookmark(&db::Move {
                repo_id: &repo.id,
                name,
                old: old.as_deref(),
                new: new.as_deref(),
                actor: &actor,
            })
            .await?;
        if let Err(db::Stale(current)) = result {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "stale",
                format!(
                    "`{name}` moved on the server (now {}); pull and try again",
                    current.as_deref().unwrap_or("absent")
                ),
            ));
        }
        self.caches.bookmarks.remove(&repo.id);
        self.metrics.bookmark_moves.fetch_add(1, Ordering::Relaxed);
        self.notify(
            repo,
            "bookmark",
            json!({ "name": name, "old": old, "new": new, "actor": actor }),
        );
        Ok(())
    }

    /// Stops live streams so replicas can drain quickly on shutdown.
    pub fn begin_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    /// Housekeeping every replica runs: forget idle rate-limit buckets,
    /// delete stale presence and expired sign-in secrets, re-queue checks
    /// whose runner vanished.
    pub fn spawn_maintenance(self: &Arc<Self>) {
        let server = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                server.limiter.prune();
                let now = db::now_ms();
                if let Err(e) = server.db.prune_presence(now - PRESENCE_TTL_MS).await {
                    tracing::warn!(error = e.message, "pruning presence failed");
                }
                if let Err(e) = server.db.prune_auth().await {
                    tracing::warn!(error = e.message, "pruning sign-in secrets failed");
                }
                if let Err(e) = server.requeue_expired_checks().await {
                    tracing::warn!(error = e.message, "re-queueing checks failed");
                }
            }
        });
    }
}

/// Drops cached bookmarks of a repository whenever any replica moves one;
/// if events were missed (slow consumer), drops them all.
fn spawn_invalidation(bus: &Bus, bookmarks: Arc<TtlCache<Arc<BTreeMap<String, String>>>>) {
    let mut events = bus.subscribe();
    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match events.recv().await {
                Ok(ev) if ev.kind == "bookmark" => bookmarks.remove(&ev.repo),
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => bookmarks.clear(),
                Err(RecvError::Closed) => break,
            }
        }
    });
}
