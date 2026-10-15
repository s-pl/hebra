//! The HTTP API: routes, middleware and the handlers, one module per area.

mod bookmarks;
mod history;
mod live;
mod presence;
mod repos;
mod reviews;
mod system;
mod transfer;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::routing::{get, post};

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

    let timeout = |d| TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, d);
    let transfer = Router::new()
        .route("/api/v1/repos/{repo}/pack", post(transfer::pack))
        .route(
            "/api/v1/repos/{repo}/objects",
            post(transfer::receive_objects).layer(DefaultBodyLimit::max(MAX_PUSH_BYTES)),
        )
        .layer(timeout(TRANSFER_TIMEOUT));
    let api = Router::new()
        .route("/api/v1/me", get(system::me))
        .route(
            "/api/v1/repos",
            get(repos::list_repos).post(repos::create_repo),
        )
        .route("/api/v1/repos/{repo}", get(repos::repo_info))
        .route(
            "/api/v1/repos/{repo}/bookmarks",
            post(bookmarks::update_bookmark),
        )
        .route("/api/v1/repos/{repo}/log", get(history::log))
        .route("/api/v1/repos/{repo}/commits/{id}", get(history::commit))
        .route("/api/v1/repos/{repo}/tree/{id}", get(history::tree))
        .route(
            "/api/v1/repos/{repo}/presence",
            get(presence::get_presence).post(presence::post_presence),
        )
        .route("/api/v1/repos/{repo}/events", get(repos::events))
        .route(
            "/api/v1/repos/{repo}/reviews",
            get(reviews::list_reviews).post(reviews::open_review),
        )
        .route(
            "/api/v1/repos/{repo}/reviews/{id}",
            get(reviews::get_review),
        )
        .route(
            "/api/v1/repos/{repo}/reviews/{id}/merge",
            post(reviews::merge_review),
        )
        .route(
            "/api/v1/repos/{repo}/reviews/{id}/close",
            post(reviews::close_review),
        )
        .layer(timeout(REQUEST_TIMEOUT))
        .layer(tower_http::compression::CompressionLayer::new());
    let live = Router::new().route("/api/v1/repos/{repo}/stream", get(live::stream));
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
                .allow_methods([Method::GET, Method::POST])
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
