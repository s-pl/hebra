//! Delivering webhooks: one replica at a time (a lease) listens to the
//! event bus and POSTs each event a webhook asked for, as JSON, signed
//! with HMAC-SHA256 of the body (`X-Hebra-Signature-256: sha256=<hex>`).
//! Failed deliveries are tried three times, waiting longer each time, and
//! every attempt is recorded.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::db::{self, Webhook};
use crate::server::Server;

const ATTEMPTS: i64 = 3;

/// The webhook event of a bus event (bookmark moves are pushes).
fn event_of(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "bookmark" => "push",
        "review" => "review",
        "task" => "task",
        "check" => "check",
        "release" => "release",
        _ => return None,
    })
}

#[must_use]
pub fn signature(secret: &[u8], body: &[u8]) -> String {
    // HMAC takes keys of any length: this cannot fail.
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return String::new();
    };
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Sends one event; returns the HTTP status of the last attempt.
pub(crate) async fn deliver(
    s: &Server,
    hook: &Webhook,
    event: &str,
    payload: &Value,
    attempts: i64,
) -> Option<i64> {
    let Ok(secret) = s.sealer.open(
        &crate::api::webhook_place(&hook.repo_id, &hook.id),
        &hook.secret,
    ) else {
        return None;
    };
    let body = payload.to_string();
    let mut last = None;
    for attempt in 1..=attempts {
        let started = Instant::now();
        let result = s
            .outbound
            .post(&hook.url)
            .timeout(Duration::from_secs(10))
            .header("Content-Type", "application/json")
            .header(
                "User-Agent",
                concat!("Hebra-Hookshot/", env!("CARGO_PKG_VERSION")),
            )
            .header("X-Hebra-Event", event)
            .header("X-Hebra-Delivery", db::new_id("dlv"))
            .header("X-Hebra-Signature-256", signature(&secret, body.as_bytes()))
            .body(body.clone())
            .send()
            .await;
        let ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let (status, error) = match result {
            Ok(r) => (Some(i64::from(r.status().as_u16())), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let _ =
            s.db.record_delivery(
                &hook.id,
                event,
                status,
                error.as_deref(),
                ms,
                attempt,
                payload,
            )
            .await;
        last = status;
        if status.is_some_and(|c| (200..300).contains(&c)) {
            break;
        }
        if attempt < attempts {
            tokio::time::sleep(Duration::from_secs(
                2u64.pow(u32::try_from(attempt).unwrap_or(1)) * 2,
            ))
            .await;
        }
    }
    last
}

impl Server {
    /// Starts the webhook dispatcher.
    pub fn spawn_webhooks(self: &Arc<Self>) {
        let server = Arc::clone(self);
        tokio::spawn(async move {
            let replica = db::new_id("hooks");
            let mut frames = server.bus.subscribe();
            let mut leader = false;
            let mut checked = Instant::now() - Duration::from_secs(60);
            loop {
                let frame = match frames.recv().await {
                    Ok(f) => f,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let Some(event) = event_of(&frame.kind) else {
                    continue;
                };
                if checked.elapsed() > Duration::from_secs(10) {
                    leader = server
                        .db
                        .acquire_lease("webhooks", &replica, 30_000)
                        .await
                        .unwrap_or(false);
                    checked = Instant::now();
                }
                if !leader {
                    continue;
                }
                let Ok(hooks) = server.db.webhooks(&frame.repo).await else {
                    continue;
                };
                let wanted: Vec<Webhook> = hooks
                    .into_iter()
                    .filter(|h| h.active && h.events.iter().any(|e| e == event || e == "*"))
                    .collect();
                if wanted.is_empty() {
                    continue;
                }
                let Ok(Some(repo)) = server.db.repo_by_id(&frame.repo).await else {
                    continue;
                };
                let data: Value = serde_json::from_str(&frame.json).unwrap_or(Value::Null);
                let payload = json!({
                    "event": event,
                    "repo": repo.full_name,
                    "data": data,
                    "at": db::rfc3339(db::now_ms()),
                });
                for hook in wanted {
                    let server = Arc::clone(&server);
                    let payload = payload.clone();
                    tokio::spawn(async move {
                        deliver(&server, &hook, event, &payload, ATTEMPTS).await;
                    });
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_are_hmac_sha256() {
        // HMAC-SHA256("key", "The quick brown fox jumps over the lazy dog").
        assert_eq!(
            signature(b"key", b"The quick brown fox jumps over the lazy dog"),
            "sha256=f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        assert_eq!(event_of("bookmark"), Some("push"));
        assert_eq!(event_of("presence"), None);
    }
}
