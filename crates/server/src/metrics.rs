//! Prometheus metrics: request counts and latency, rate limiting, pushes,
//! bookmark moves, merges and live subscribers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::server::Server;

const BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Default)]
pub struct Metrics {
    by_class: [AtomicU64; 5],
    duration_buckets: [AtomicU64; BUCKETS.len()],
    duration_count: AtomicU64,
    duration_micros: AtomicU64,
    pub rate_limited: AtomicU64,
    pub objects_received: AtomicU64,
    pub bookmark_moves: AtomicU64,
    pub merges: AtomicU64,
}

impl Metrics {
    fn observe(&self, status: StatusCode, seconds: f64) {
        let class = usize::from(status.as_u16() / 100).clamp(1, 5) - 1;
        self.by_class[class].fetch_add(1, Ordering::Relaxed);
        for (i, upper) in BUCKETS.iter().enumerate() {
            if seconds <= *upper {
                self.duration_buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.duration_count.fetch_add(1, Ordering::Relaxed);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        self.duration_micros
            .fetch_add((seconds * 1e6) as u64, Ordering::Relaxed);
    }

    /// Prometheus text exposition.
    #[must_use]
    pub fn render(&self, server: &crate::Server) -> String {
        use std::fmt::Write;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# HELP hebra_info Build and backend information.\n# TYPE hebra_info gauge"
        );
        let _ = writeln!(
            out,
            "hebra_info{{version=\"{}\",meta=\"{}\",bus=\"{}\",encrypted=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION"),
            server.meta.backend(),
            server.bus.backend(),
            server.objects.encrypted()
        );
        let _ = writeln!(
            out,
            "# HELP hebra_http_requests_total HTTP responses by status class.\n# TYPE hebra_http_requests_total counter"
        );
        for (i, class) in ["1xx", "2xx", "3xx", "4xx", "5xx"].iter().enumerate() {
            let _ = writeln!(
                out,
                "hebra_http_requests_total{{class=\"{class}\"}} {}",
                get(&self.by_class[i])
            );
        }
        let _ = writeln!(
            out,
            "# HELP hebra_http_request_duration_seconds Time to first response byte.\n# TYPE hebra_http_request_duration_seconds histogram"
        );
        for (i, upper) in BUCKETS.iter().enumerate() {
            let _ = writeln!(
                out,
                "hebra_http_request_duration_seconds_bucket{{le=\"{upper}\"}} {}",
                get(&self.duration_buckets[i])
            );
        }
        let count = get(&self.duration_count);
        let _ = writeln!(
            out,
            "hebra_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {count}"
        );
        #[allow(clippy::cast_precision_loss)]
        let sum = get(&self.duration_micros) as f64 / 1e6;
        let _ = writeln!(out, "hebra_http_request_duration_seconds_sum {sum}");
        let _ = writeln!(out, "hebra_http_request_duration_seconds_count {count}");
        for (name, help, value) in [
            (
                "hebra_rate_limited_total",
                "Requests refused by the rate limiter.",
                get(&self.rate_limited),
            ),
            (
                "hebra_objects_received_total",
                "Objects received in pushes.",
                get(&self.objects_received),
            ),
            (
                "hebra_bookmark_moves_total",
                "Bookmarks moved.",
                get(&self.bookmark_moves),
            ),
            (
                "hebra_review_merges_total",
                "Reviews merged by the server.",
                get(&self.merges),
            ),
        ] {
            let _ = writeln!(
                out,
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}"
            );
        }
        let _ = writeln!(
            out,
            "# HELP hebra_live_subscribers Open live streams on this replica.\n# TYPE hebra_live_subscribers gauge\nhebra_live_subscribers {}",
            server.bus.subscribers()
        );
        out
    }
}

pub async fn track(State(state): State<Arc<Server>>, request: Request, next: Next) -> Response {
    let start = Instant::now();
    let response = next.run(request).await;
    state
        .metrics
        .observe(response.status(), start.elapsed().as_secs_f64());
    response
}
