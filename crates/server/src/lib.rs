//! Hebra server: hosts repositories for people and agents, and serves the
//! HTTP API used by `hb`, MCP clients, runners and the web app.
//!
//! Stateless by design, so it scales horizontally:
//! - metadata in SQLite (one node) or PostgreSQL (many), one implementation
//!   with versioned migrations ([`db`]);
//! - objects in a directory or S3-compatible storage, encrypted at rest with
//!   XChaCha20-Poly1305 ([`objects`]);
//! - live events fanned out between replicas over NATS ([`bus`]).
//!
//! Layout:
//! - [`identity`]: who is calling (web sessions, mandates) and the single
//!   place that decides what they may do (`Server::authorize`);
//! - `api`: routes and handlers, one module per area;
//! - [`checks`]: `.hb/checks.toml` and `.hb/policy.toml`, triggers,
//!   leases; `gateway`: the model gateway for agent verification;
//! - [`billing`]: plans, quotas, usage and Stripe (Cloud);
//! - [`ratelimit`], [`headers`], [`metrics`]: middleware;
//! - [`serve`]: HTTP / HTTPS listeners with graceful shutdown.

mod api;
pub mod billing;
pub mod bus;
mod cache;
pub mod checks;
pub mod config;
pub mod db;
mod error;
mod gateway;
pub mod headers;
pub mod identity;
mod merging;
pub mod metrics;
pub mod names;
pub mod objects;
pub mod ratelimit;
pub mod serve;
mod server;

/// Header types, for callers outside the server (benchmarks).
pub use axum::http;

pub use api::{metrics_router, router};
pub use bus::{Bus, LiveEvent, NatsConfig};
pub use config::{Config, Edition, GithubOauth, ModelGateway, Signup, Stripe};
pub use error::ApiError;
pub use names::glob_match;
pub use objects::{Objects, ObjectsConfig};
pub use ratelimit::{Limits, RateLimiter};
pub use serve::{serve, serve_tls_until, serve_until, shutdown_signal};
pub use server::Server;
