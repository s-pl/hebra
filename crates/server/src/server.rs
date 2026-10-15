//! The server state shared by every request: metadata, objects, the event
//! bus, and the per-replica caches in front of them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode, header};
use hebra_core::ObjectId;
use hebra_core::store::Store;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::api::blocking;
use crate::auth::{TokenInfo, glob_match, hash_token, valid_bookmark, valid_repo_name};
use crate::bus::{Bus, LiveEvent};
use crate::cache::TtlCache;
use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::meta::{self, Meta, TokenRow};
use crate::metrics::Metrics;
use crate::objects::Objects;
use crate::ratelimit::RateLimiter;

/// Presence entries older than this are hidden (and later deleted).
pub(crate) const PRESENCE_TTL_SECS: i64 = 15 * 60;
/// How long a replica trusts a token check or a repository's visibility
/// before asking the database again (so a revoked token stops working
/// everywhere within this time).
const AUTH_TTL: Duration = Duration::from_secs(5);
const NEGATIVE_TTL: Duration = Duration::from_secs(1);
/// Upper bound for a cached bookmark map if an invalidation is ever lost.
const BOOKMARKS_TTL: Duration = Duration::from_secs(30);

pub(crate) fn now() -> String {
    jiff::Timestamp::now().to_string()
}

pub(crate) fn unix_now() -> i64 {
    jiff::Timestamp::now().as_second()
}

pub struct Server {
    pub(crate) meta: Meta,
    pub(crate) objects: Objects,
    pub(crate) bus: Bus,
    pub(crate) limiter: RateLimiter,
    pub(crate) metrics: Metrics,
    pub(crate) hsts: bool,
    pub(crate) cors_origins: Vec<String>,
    /// Token hash -> token (`None`: unknown), trusted for `AUTH_TTL`.
    tokens: TtlCache<Option<TokenInfo>>,
    /// Repository -> public? (`None`: does not exist), trusted for `AUTH_TTL`.
    visibility: TtlCache<Option<bool>>,
    /// Repository -> bookmarks, for reads. Dropped when a bookmark event
    /// for the repository arrives (from any replica, through the bus).
    bookmarks: Arc<TtlCache<Arc<BTreeMap<String, String>>>>,
    /// Repository -> protected patterns, trusted for `AUTH_TTL`.
    protected: TtlCache<Arc<Vec<String>>>,
    pub(crate) shutdown: watch::Sender<bool>,
}

impl Server {
    pub async fn open(config: Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data)?;
        let meta = match &config.database_url {
            Some(url) => Meta::postgres(url, config.db_max_connections).await?,
            None => Meta::sqlite(&config.data.join("hebra-server.db"))?,
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
        let bookmarks = Arc::new(TtlCache::new(10_000));
        spawn_invalidation(&bus, Arc::clone(&bookmarks));
        Ok(Self {
            meta,
            objects,
            bus,
            limiter: RateLimiter::new(config.limits, config.trust_proxy),
            metrics: Metrics::default(),
            hsts: config.hsts,
            cors_origins: config.cors_origins,
            tokens: TtlCache::new(100_000),
            visibility: TtlCache::new(100_000),
            bookmarks,
            protected: TtlCache::new(100_000),
            shutdown: watch::channel(false).0,
        })
    }

    /// One line describing the backends, for startup logs.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "metadata: {}, events: {}, objects encrypted: {}",
            self.meta.backend(),
            self.bus.backend(),
            if self.objects.encrypted() {
                "yes"
            } else {
                "no"
            }
        )
    }

    pub(crate) fn store(&self, repo: &str) -> Store {
        self.objects.store(repo)
    }

    pub async fn create_repo(&self, name: &str, protect: &[String]) -> anyhow::Result<()> {
        anyhow::ensure!(valid_repo_name(name), "invalid repository name `{name}`");
        for p in protect {
            anyhow::ensure!(
                valid_bookmark(&p.replace('*', "x")),
                "invalid pattern `{p}`"
            );
        }
        self.objects.create(name)?;
        let created = self
            .meta
            .create_repo(name, protect, &now())
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        anyhow::ensure!(created, "repository `{name}` already exists");
        self.visibility.remove(name);
        Ok(())
    }

    /// Creates a token and returns its secret (shown once; only a hash is
    /// stored). With `ttl`, it stops working after that long.
    pub async fn create_token(
        &self,
        info: &TokenInfo,
        ttl: Option<Duration>,
    ) -> anyhow::Result<String> {
        let mut secret = [0u8; 24];
        rand::fill(&mut secret);
        let token = format!("hbt_{}", hex::encode(secret));
        let expires = ttl.map(|t| unix_now() + i64::try_from(t.as_secs()).unwrap_or(i64::MAX / 2));
        self.meta
            .insert_token(&hash_token(&token), info, &now(), expires)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        Ok(token)
    }

    pub async fn list_tokens(&self) -> anyhow::Result<Vec<TokenRow>> {
        self.meta
            .tokens()
            .await
            .map_err(|e| anyhow::anyhow!(e.message))
    }

    pub async fn revoke_tokens(&self, name: &str) -> anyhow::Result<u64> {
        self.meta
            .revoke_tokens(name)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))
    }

    pub(crate) async fn token(&self, headers: &HeaderMap) -> ApiResult<Option<TokenInfo>> {
        let Some(value) = headers.get(header::AUTHORIZATION) else {
            return Ok(None);
        };
        let value = value
            .to_str()
            .map_err(|_| ApiError::unauthorized("malformed authorization header"))?;
        let token = value.strip_prefix("Bearer ").unwrap_or(value).trim();
        if !token.starts_with("hbt_") || token.len() > 128 {
            return Err(ApiError::unauthorized("unknown or expired token"));
        }
        let hash = hash_token(token);
        let found = match self.tokens.get(&hash) {
            Some(cached) => cached,
            None => {
                let found = self.meta.token(&hash, unix_now()).await?;
                let ttl = if found.is_some() {
                    AUTH_TTL
                } else {
                    NEGATIVE_TTL
                };
                self.tokens.put(&hash, found.clone(), ttl);
                found
            }
        };
        found
            .map(Some)
            .ok_or_else(|| ApiError::unauthorized("unknown or expired token"))
    }

    pub(crate) async fn require_token(&self, headers: &HeaderMap) -> ApiResult<TokenInfo> {
        self.token(headers)
            .await?
            .ok_or_else(|| ApiError::unauthorized("a token is required"))
    }

    pub(crate) async fn require_repo(&self, repo: &str, headers: &HeaderMap) -> ApiResult<()> {
        let public = match self.visibility.get(repo) {
            Some(cached) => cached,
            None => {
                let public = self.meta.repo_public(repo).await?;
                let ttl = if public.is_some() {
                    AUTH_TTL
                } else {
                    NEGATIVE_TTL
                };
                self.visibility.put(repo, public, ttl);
                public
            }
        };
        match public {
            None => Err(ApiError::not_found(format!(
                "repository `{repo}` not found"
            ))),
            Some(true) => Ok(()),
            Some(false) => self.require_token(headers).await.map(|_| ()),
        }
    }

    /// Tells live subscribers of `repo` on every replica.
    pub(crate) fn notify(&self, repo: &str, kind: &str, data: Value) {
        self.bus.publish(LiveEvent {
            repo: repo.to_owned(),
            kind: kind.to_owned(),
            data,
        });
    }

    /// All bookmarks of `repo`, for reads (may lag a write on another
    /// replica by the time its event takes to arrive).
    pub(crate) async fn bookmarks(&self, repo: &str) -> ApiResult<Arc<BTreeMap<String, String>>> {
        if let Some(cached) = self.bookmarks.get(repo) {
            return Ok(cached);
        }
        let fresh = Arc::new(self.meta.bookmarks(repo).await?);
        self.bookmarks.put(repo, Arc::clone(&fresh), BOOKMARKS_TTL);
        Ok(fresh)
    }

    pub(crate) async fn bookmark(&self, repo: &str, name: &str) -> ApiResult<Option<ObjectId>> {
        self.bookmarks(repo)
            .await?
            .get(name)
            .map(|t| ObjectId::parse(t).map_err(ApiError::from))
            .transpose()
    }

    /// A bookmark straight from the database, for decisions that write.
    pub(crate) async fn bookmark_fresh(
        &self,
        repo: &str,
        name: &str,
    ) -> ApiResult<Option<ObjectId>> {
        self.meta
            .bookmark(repo, name)
            .await?
            .map(|t| ObjectId::parse(&t).map_err(ApiError::from))
            .transpose()
    }

    pub(crate) async fn protected(&self, repo: &str) -> ApiResult<Arc<Vec<String>>> {
        if let Some(cached) = self.protected.get(repo) {
            return Ok(cached);
        }
        let fresh = Arc::new(self.meta.protected(repo).await?);
        self.protected.put(repo, Arc::clone(&fresh), AUTH_TTL);
        Ok(fresh)
    }

    pub(crate) async fn resolve(self: &Arc<Self>, repo: &str, rev: &str) -> ApiResult<ObjectId> {
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

    /// Permission to move `name`: allowed by the token's patterns, and never
    /// a protected bookmark for agents.
    pub(crate) async fn check_move(
        &self,
        repo: &str,
        token: &TokenInfo,
        name: &str,
    ) -> ApiResult<()> {
        if !token.allow.iter().any(|p| glob_match(p, name)) {
            return Err(ApiError::forbidden(
                "forbidden",
                format!(
                    "token `{}` may not move `{name}` (allowed: {})",
                    token.name,
                    token.allow.join(", ")
                ),
            ));
        }
        let protected = self
            .protected(repo)
            .await?
            .iter()
            .any(|p| glob_match(p, name));
        if protected && token.kind == "agent" {
            return Err(ApiError::forbidden(
                "needs_human",
                format!(
                    "`{name}` is protected: agents push to their own bookmarks and a person moves `{name}`"
                ),
            ));
        }
        Ok(())
    }

    /// Compare-and-swap a bookmark (atomic across replicas), record the event
    /// and tell live clients.
    pub(crate) async fn move_bookmark(
        &self,
        repo: &str,
        token: &TokenInfo,
        name: &str,
        old: Option<String>,
        new: Option<String>,
    ) -> ApiResult<()> {
        let actor = token.display();
        let result = self
            .meta
            .cas_bookmark(&meta::Move {
                repo,
                name,
                old: old.as_deref(),
                new: new.as_deref(),
                actor: &actor,
                at: &now(),
            })
            .await?;
        if let Err(meta::Stale(current)) = result {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "stale",
                format!(
                    "`{name}` moved on the server (now {}); pull and try again",
                    current.as_deref().unwrap_or("absent")
                ),
            ));
        }
        self.bookmarks.remove(repo);
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

    /// Housekeeping every replica runs: forget idle rate-limit buckets and
    /// delete stale presence.
    pub fn spawn_maintenance(self: &Arc<Self>) {
        let server = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                server.limiter.prune();
                if let Err(e) = server
                    .meta
                    .prune_presence(unix_now() - PRESENCE_TTL_SECS)
                    .await
                {
                    tracing::warn!(error = e.message, "pruning presence failed");
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
