//! Hebra server: hosts repositories and serves the HTTP API used by `hb push`,
//! `hb pull`, presence, reviews and the web app.
//!
//! Stateless by design, so it scales horizontally:
//! - metadata (repositories, bookmarks, tokens, presence, events, reviews) in
//!   SQLite for one node or PostgreSQL for many ([`meta`]);
//! - objects in a directory or S3-compatible storage, encrypted at rest with
//!   XChaCha20-Poly1305 ([`objects`]);
//! - live events fanned out between replicas over NATS ([`bus`]).
//!
//! Layout:
//! - [`server`]: the shared state and the operations handlers build on
//!   (authentication, permissions, bookmark moves, caches);
//! - `api`: routes and handlers, one module per area;
//! - [`ratelimit`], [`headers`], [`metrics`]: middleware;
//! - `merging`: what merging a review would do, and doing it;
//! - [`serve`]: HTTP / HTTPS listeners with graceful shutdown.
//!
//! Permissions: every write needs a token. A token may move only the
//! bookmarks matching its patterns, and agent tokens can never move a
//! protected bookmark (by default `main`): a person has to do that.

mod api;
mod auth;
pub mod bus;
mod cache;
mod config;
mod error;
pub mod headers;
mod merging;
pub mod meta;
pub mod metrics;
pub mod objects;
pub mod ratelimit;
pub mod serve;
mod server;

/// Header types, for callers outside the server (benchmarks).
pub use axum::http;

pub use api::{metrics_router, router};
pub use auth::{TokenInfo, glob_match, valid_repo_name};
pub use bus::{Bus, LiveEvent, NatsConfig};
pub use config::Config;
pub use error::ApiError;
pub use meta::{Meta, TokenRow};
pub use objects::{Objects, ObjectsConfig};
pub use ratelimit::{Limits, RateLimiter};
pub use serve::{serve, serve_tls_until, serve_until, shutdown_signal};
pub use server::Server;
