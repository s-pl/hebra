//! Live events between replicas.
//!
//! Each replica keeps a local broadcast channel that its SSE clients listen
//! to. With one replica that is all there is. With several, events are
//! published to NATS (`hebra.live.<repo>`) and every replica forwards what it
//! receives into its own channel, so a push handled by one pod reaches
//! browsers and agents connected to any other. A JetStream stream keeps the
//! same subjects for a week, for audit and for consumers such as webhooks.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

/// Something that just happened in a repository, sent to live subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveEvent {
    pub repo: String,
    pub kind: String,
    pub data: Value,
}

#[derive(Debug, Clone, Default)]
pub struct NatsConfig {
    /// `nats://…` or `tls://…`; several separated by commas.
    pub url: String,
    /// `.creds` file (JWT + NKey) for decentralised auth.
    pub creds: Option<PathBuf>,
    /// CA bundle to verify the NATS servers.
    pub ca: Option<PathBuf>,
    /// Client certificate and key for mutual TLS.
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
}

/// An event as live subscribers receive it: serialised once and shared,
/// however many streams carry it.
#[derive(Debug)]
pub struct Frame {
    pub repo: String,
    pub kind: String,
    pub json: String,
}

impl Frame {
    fn of(event: &LiveEvent) -> Arc<Self> {
        Arc::new(Self {
            repo: event.repo.clone(),
            kind: event.kind.clone(),
            json: event.data.to_string(),
        })
    }
}

/// Events a replica buffers per subscriber before telling it to resync.
const BUFFER: usize = 16_384;

const STREAM: &str = "HEBRA_EVENTS";
const PREFIX: &str = "hebra.live";

pub struct Bus {
    local: broadcast::Sender<Arc<Frame>>,
    nats: Option<async_nats::Client>,
    /// Events waiting to be published, in order, by one task.
    outbox: Option<mpsc::Sender<LiveEvent>>,
}

/// NATS subjects split on dots; repository names may contain them.
fn subject(repo: &str) -> String {
    format!("{PREFIX}.{}", repo.replace('.', "_"))
}

impl Bus {
    #[must_use]
    pub fn local() -> Self {
        let (local, _) = broadcast::channel(BUFFER);
        Self {
            local,
            nats: None,
            outbox: None,
        }
    }

    pub async fn nats(config: &NatsConfig) -> anyhow::Result<Self> {
        let mut options = async_nats::ConnectOptions::new()
            .name(format!("hebra-server {}", env!("CARGO_PKG_VERSION")))
            .retry_on_initial_connect()
            .connection_timeout(Duration::from_secs(10));
        if let Some(creds) = &config.creds {
            options = options.credentials_file(creds).await?;
        }
        if let Some(ca) = &config.ca {
            options = options.add_root_certificates(ca.clone()).require_tls(true);
        }
        if let (Some(cert), Some(key)) = (&config.cert, &config.key) {
            options = options
                .add_client_certificate(cert.clone(), key.clone())
                .require_tls(true);
        }
        if config.url.starts_with("tls://") {
            options = options.require_tls(true);
        }
        let client = options.connect(config.url.as_str()).await?;

        let jetstream = async_nats::jetstream::new(client.clone());
        jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: STREAM.to_owned(),
                subjects: vec![format!("{PREFIX}.>")],
                max_age: Duration::from_secs(7 * 24 * 3600),
                ..Default::default()
            })
            .await?;

        let (local, _) = broadcast::channel(BUFFER);
        let mut sub = client.subscribe(format!("{PREFIX}.>")).await?;
        let forward = local.clone();
        tokio::spawn(async move {
            use tokio_stream::StreamExt;
            while let Some(msg) = sub.next().await {
                match serde_json::from_slice::<LiveEvent>(&msg.payload) {
                    Ok(ev) => {
                        let _ = forward.send(Frame::of(&ev));
                    }
                    Err(e) => tracing::warn!(error = %e, "ignoring malformed live event"),
                }
            }
            tracing::error!("NATS subscription ended");
        });
        let (outbox, mut queue) = mpsc::channel::<LiveEvent>(4096);
        let publisher = client.clone();
        let fallback = local.clone();
        tokio::spawn(async move {
            while let Some(event) = queue.recv().await {
                let Ok(payload) = serde_json::to_vec(&event) else {
                    continue;
                };
                if let Err(e) = publisher
                    .publish(subject(&event.repo), payload.into())
                    .await
                {
                    tracing::warn!(error = %e, "NATS publish failed; delivering locally");
                    let _ = fallback.send(Frame::of(&event));
                }
            }
        });
        Ok(Self {
            local,
            nats: Some(client),
            outbox: Some(outbox),
        })
    }

    #[must_use]
    pub fn backend(&self) -> &'static str {
        if self.nats.is_some() { "nats" } else { "local" }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Frame>> {
        self.local.subscribe()
    }

    #[must_use]
    pub fn subscribers(&self) -> usize {
        self.local.receiver_count()
    }

    /// Fire and forget: with NATS the event comes back through the
    /// subscription (to this replica too); if NATS is unreachable or the
    /// outbox is full, local clients still hear about it.
    pub fn publish(&self, event: LiveEvent) {
        let Some(outbox) = &self.outbox else {
            let _ = self.local.send(Frame::of(&event));
            return;
        };
        if let Err(e) = outbox.try_send(event) {
            tracing::warn!("live event outbox full; delivering locally");
            let _ = self.local.send(Frame::of(&e.into_inner()));
        }
    }

    /// Whether NATS is connected (readiness).
    #[must_use]
    pub fn healthy(&self) -> bool {
        self.nats
            .as_ref()
            .is_none_or(|c| c.connection_state() == async_nats::connection::State::Connected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_have_one_token_per_repo() {
        assert_eq!(subject("acme"), "hebra.live.acme");
        assert_eq!(subject("acme.web"), "hebra.live.acme_web");
    }
}
