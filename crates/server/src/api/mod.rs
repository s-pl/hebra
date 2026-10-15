//! The HTTP API: routes, middleware, and the handlers, one module per area.
//!
//! Repository routes live under `/api/v1/repos/{owner}/{repo}`. Every
//! handler authorizes through `Server::access` / `Server::authorize`.

mod accounts;
mod auth;
mod billing;
mod bookmarks;
mod checks;
mod history;
mod live;
mod presence;
mod repos;
mod reviews;
mod runner;
mod system;
mod tasks;
mod transfer;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::routing::{delete, get, post, put};

use crate::error::{ApiError, ApiResult};
use crate::server::Server;
use crate::{headers, metrics, ratelimit};

/// Largest push accepted in one request.
const MAX_PUSH_BYTES: usize = 1024 * 1024 * 1024;
/// Every request except live streams and object transfer must finish in this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(crate) type St = State<Arc<Server>>;

/// Runs blocking work (object reads and writes) off the async runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::internal("internal", e))?
}

pub fn router(server: Arc<Server>) -> Router {
    use axum::middleware::from_fn_with_state;
    use tower_http::timeout::TimeoutLayer;

    const REPO: &str = "/api/v1/repos/{owner}/{repo}";
    let r = |path: &str| format!("{REPO}{path}");
    let timeout = |d| TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, d);

    let transfer = Router::new()
        .route(&r("/pack"), post(transfer::pack))
        .route(
            &r("/objects"),
            post(transfer::receive_objects).layer(DefaultBodyLimit::max(MAX_PUSH_BYTES)),
        )
        .layer(timeout(TRANSFER_TIMEOUT));

    let api = Router::new()
        // Platform
        .route("/api/v1/capabilities", get(system::capabilities))
        .route("/api/v1/plans", get(billing::plans))
        .route("/api/v1/transparency/key", get(system::transparency_key))
        // Sign-in
        .route("/api/v1/auth/github", post(auth::github))
        .route("/api/v1/auth/invite", post(auth::invite))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/device", post(auth::device_start))
        .route("/api/v1/auth/device/token", post(auth::device_token))
        .route(
            "/api/v1/auth/device/{code}",
            get(auth::device_show).post(auth::device_decide),
        )
        // People, organizations, agents
        .route("/api/v1/me", get(accounts::me))
        .route(
            "/api/v1/me/mandates",
            get(accounts::my_mandates).post(accounts::issue_mandate),
        )
        .route("/api/v1/me/mandates/{id}", delete(accounts::revoke_mandate))
        .route("/api/v1/orgs", post(accounts::create_org))
        .route("/api/v1/accounts/{handle}", get(accounts::profile))
        .route("/api/v1/accounts/{handle}/repos", get(repos::list_owned))
        .route("/api/v1/accounts/{handle}/members", get(accounts::members))
        .route(
            "/api/v1/accounts/{handle}/members/{member}",
            put(accounts::set_member).delete(accounts::remove_member),
        )
        .route(
            "/api/v1/accounts/{handle}/teams",
            get(accounts::teams).post(accounts::create_team),
        )
        .route(
            "/api/v1/accounts/{handle}/teams/{team}",
            delete(accounts::delete_team),
        )
        .route(
            "/api/v1/accounts/{handle}/teams/{team}/members/{member}",
            put(accounts::add_team_member).delete(accounts::remove_team_member),
        )
        .route(
            "/api/v1/accounts/{handle}/agents",
            get(accounts::agents).post(accounts::create_agent),
        )
        .route(
            "/api/v1/accounts/{handle}/agents/{agent}",
            delete(accounts::disable_agent),
        )
        .route(
            "/api/v1/accounts/{handle}/agents/{agent}/mandates",
            post(accounts::agent_mandate),
        )
        .route(
            "/api/v1/accounts/{handle}/runners",
            get(runner::list).post(runner::register),
        )
        .route(
            "/api/v1/accounts/{handle}/runners/{name}",
            delete(runner::remove),
        )
        .route("/api/v1/accounts/{handle}/billing", get(billing::overview))
        .route(
            "/api/v1/accounts/{handle}/billing/checkout",
            post(billing::checkout),
        )
        .route(
            "/api/v1/accounts/{handle}/billing/portal",
            post(billing::portal),
        )
        .route(
            "/api/v1/accounts/{handle}/billing/limit",
            put(billing::set_limit),
        )
        .route("/api/v1/billing/stripe", post(billing::stripe_webhook))
        // Runners
        .route("/api/v1/runner/claim", post(runner::claim))
        .route(
            "/api/v1/runner/runs/{id}/heartbeat",
            post(runner::heartbeat),
        )
        .route("/api/v1/runner/runs/{id}/log", post(runner::log))
        .route("/api/v1/runner/runs/{id}/complete", post(runner::complete))
        .route("/api/v1/runner/runs/{id}/agent", post(runner::agent))
        // Repositories
        .route("/api/v1/repos", get(repos::list).post(repos::create))
        .route(
            REPO,
            get(repos::info).patch(repos::update).delete(repos::remove),
        )
        .route(&r("/access"), get(repos::access))
        .route(
            &r("/access/{principal}"),
            put(repos::grant).delete(repos::revoke),
        )
        .route(&r("/fork"), post(repos::fork))
        .route(&r("/events"), get(repos::events))
        .route(&r("/transparency"), get(repos::transparency))
        .route(&r("/queue"), get(reviews::queue))
        .route(&r("/bookmarks"), post(bookmarks::update))
        .route(&r("/log"), get(history::log))
        .route(&r("/commits/{id}"), get(history::commit))
        .route(&r("/tree/{id}"), get(history::tree))
        .route(&r("/presence"), get(presence::get).post(presence::post))
        // Reviews
        .route(&r("/reviews"), get(reviews::list).post(reviews::open))
        .route(&r("/reviews/{number}"), get(reviews::show))
        .route(&r("/reviews/{number}/comments"), post(reviews::comment))
        .route(
            &r("/reviews/{number}/comments/{comment}/resolve"),
            post(reviews::resolve),
        )
        .route(&r("/reviews/{number}/approve"), post(reviews::approve))
        .route(&r("/reviews/{number}/merge"), post(reviews::merge))
        .route(&r("/reviews/{number}/close"), post(reviews::close))
        // Tasks
        .route(&r("/tasks"), get(tasks::list).post(tasks::create))
        .route(&r("/tasks/{number}"), get(tasks::show).patch(tasks::edit))
        .route(&r("/tasks/{number}/claim"), post(tasks::claim))
        .route(&r("/tasks/{number}/release"), post(tasks::release))
        .route(&r("/tasks/{number}/close"), post(tasks::close))
        .route(&r("/tasks/{number}/reopen"), post(tasks::reopen))
        // Checks
        .route(&r("/checks"), get(checks::list))
        .route(&r("/checks/approve"), post(checks::approve))
        .route(&r("/checks/rerun"), post(checks::rerun))
        .route(&r("/checks/{id}"), get(checks::show))
        .route(&r("/checks/{id}/log"), get(checks::log))
        .route(&r("/checks/{id}/cancel"), post(checks::cancel))
        .layer(timeout(REQUEST_TIMEOUT))
        .layer(tower_http::compression::CompressionLayer::new());
    let live = Router::new().route(&r("/stream"), get(live::stream));
    // Probes (kubelet, load balancers) are not rate limited.
    let probes = Router::new()
        .route("/api/v1/health", get(system::health))
        .route("/api/v1/ready", get(system::ready));

    let mut app = api
        .merge(transfer)
        .merge(live)
        .layer(from_fn_with_state(
            Arc::clone(&server),
            ratelimit::rate_limit,
        ))
        .merge(probes)
        .layer(from_fn_with_state(
            Arc::clone(&server),
            headers::security_headers,
        ))
        .layer(from_fn_with_state(Arc::clone(&server), metrics::track))
        .layer(tower_http::trace::TraceLayer::new_for_http());
    if !server.cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = server
            .cors_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        app = app.layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                ])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                .max_age(Duration::from_secs(600)),
        );
    }
    app.with_state(server)
}

/// The metrics endpoint, served on its own (internal) port.
pub fn metrics_router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/metrics", get(system::metrics))
        .with_state(server)
}
