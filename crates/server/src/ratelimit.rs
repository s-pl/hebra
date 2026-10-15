//! Per-client rate limits: token buckets keyed by client IP and by API
//! token, so made-up tokens cannot buy fresh buckets.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::server::Server;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Sustained requests per second per API token.
    pub token_rps: f64,
    pub token_burst: f64,
    /// Sustained requests per second per client IP, with or without a
    /// token (so made-up tokens cannot buy fresh buckets).
    pub ip_rps: f64,
    pub ip_burst: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            token_rps: 100.0,
            token_burst: 1000.0,
            ip_rps: 200.0,
            ip_burst: 2000.0,
        }
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

const SHARDS: usize = 32;

/// Token buckets keyed by API token (hashed) or client IP.
pub struct RateLimiter {
    limits: Limits,
    /// Use `X-Real-IP` from the ingress instead of the TCP peer.
    trust_proxy: bool,
    shards: Vec<Mutex<HashMap<String, Bucket>>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(limits: Limits, trust_proxy: bool) -> Self {
        Self {
            limits,
            trust_proxy,
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
        }
    }

    fn ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
        let forwarded = self
            .trust_proxy
            .then(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
            .flatten()
            .map(str::to_owned);
        forwarded
            .or_else(|| peer.map(|p| p.ip().to_string()))
            .unwrap_or_else(|| "unknown".to_owned())
    }

    /// Takes one request from the bucket; `Err(retry_after_secs)` when empty.
    fn take(&self, key: &str, rate: f64, burst: f64) -> Result<(), u64> {
        let shard = &self.shards[usize::from(blake3::hash(key.as_bytes()).as_bytes()[0]) % SHARDS];
        let mut map = shard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let bucket = map.entry(key.to_owned()).or_insert(Bucket {
            tokens: burst,
            at: now,
        });
        let elapsed = now.duration_since(bucket.at).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rate).min(burst);
        bucket.at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let wait = ((1.0 - bucket.tokens) / rate).ceil() as u64;
            Err(wait.max(1))
        }
    }

    /// Charges the client's IP and, if it sent one, its token.
    pub fn check(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> Result<(), u64> {
        let l = self.limits;
        self.take(
            &format!("ip:{}", self.ip(headers, peer)),
            l.ip_rps,
            l.ip_burst,
        )?;
        if let Some(auth) = headers.get(header::AUTHORIZATION) {
            let hash = blake3::hash(auth.as_bytes()).to_hex();
            self.take(&format!("t:{}", &hash[..32]), l.token_rps, l.token_burst)?;
        }
        Ok(())
    }

    /// Forgets clients whose buckets are full again (call periodically).
    pub fn prune(&self) {
        let now = Instant::now();
        let full_after = (self.limits.token_burst / self.limits.token_rps)
            .max(self.limits.ip_burst / self.limits.ip_rps);
        for shard in &self.shards {
            let mut map = shard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            map.retain(|_, b| now.duration_since(b.at).as_secs_f64() < full_after);
        }
    }
}

pub async fn rate_limit(
    State(state): State<Arc<Server>>,
    request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    match state.limiter.check(request.headers(), peer) {
        Ok(()) => next.run(request).await,
        Err(retry) => {
            state.metrics.rate_limited.fetch_add(1, Ordering::Relaxed);
            let body = json!({ "error": { "code": "rate_limited", "message": format!("too many requests; retry in {retry}s") } });
            let mut response = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
            if let Ok(v) = HeaderValue::from_str(&retry.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, v);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_refill_and_refuse() {
        let limiter = RateLimiter::new(
            Limits {
                token_rps: 1.0,
                token_burst: 2.0,
                ip_rps: 1.0,
                ip_burst: 3.0,
            },
            false,
        );
        let peer = Some("10.0.0.1:5000".parse().unwrap());
        let mut with_token = HeaderMap::new();
        with_token.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer a"));
        assert!(limiter.check(&with_token, peer).is_ok());
        assert!(limiter.check(&with_token, peer).is_ok());
        // The token is out; the IP still has one left.
        assert!(limiter.check(&with_token, peer).is_err());
        assert!(limiter.check(&HeaderMap::new(), peer).is_err());
        // Other clients are unaffected.
        assert!(
            limiter
                .check(&HeaderMap::new(), Some("10.0.0.2:1".parse().unwrap()))
                .is_ok()
        );
    }

    #[test]
    fn proxy_header_only_when_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("203.0.113.9"));
        let peer = Some("10.0.0.1:5000".parse().unwrap());
        let direct = RateLimiter::new(Limits::default(), false);
        assert_eq!(direct.ip(&headers, peer), "10.0.0.1");
        let proxied = RateLimiter::new(Limits::default(), true);
        assert_eq!(proxied.ip(&headers, peer), "203.0.113.9");
    }
}
