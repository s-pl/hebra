//! Security headers on every API response.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::server::Server;

/// Headers for an API that never serves HTML: nothing may be framed, sniffed,
/// cached or leak a referrer. HSTS is added when the server (or its ingress)
/// speaks HTTPS.
pub async fn security_headers(
    State(state): State<Arc<Server>>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    let set = |h: &mut HeaderMap, name: header::HeaderName, value: &'static str| {
        h.entry(name).or_insert(HeaderValue::from_static(value));
    };
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(
        h,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'",
    );
    set(h, header::CACHE_CONTROL, "no-store");
    h.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-site"),
    );
    if state.hsts {
        set(
            h,
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=63072000; includeSubDomains; preload",
        );
    }
    response
}
