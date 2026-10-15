//! Listening: plain HTTP or HTTPS, with graceful shutdown.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::api::router;
use crate::server::Server;

/// Waits for Ctrl+C or SIGTERM (Kubernetes stopping the pod).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}

/// Serves plain HTTP until the process is stopped.
pub async fn serve(server: Arc<Server>, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    serve_until(server, listener, std::future::pending()).await
}

/// Serves plain HTTP until `stop` completes, then drains connections.
pub async fn serve_until(
    server: Arc<Server>,
    listener: tokio::net::TcpListener,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let app = router(Arc::clone(&server));
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        stop.await;
        server.begin_shutdown();
    })
    .await?;
    Ok(())
}

/// Serves HTTPS (TLS 1.2/1.3 via rustls) until `stop` completes. The
/// certificate is reloaded from disk every 10 minutes, so rotations by
/// cert-manager apply without restarts.
pub async fn serve_tls_until(
    server: Arc<Server>,
    addr: SocketAddr,
    cert: PathBuf,
    key: PathBuf,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    use axum_server::tls_rustls::RustlsConfig;
    let config = RustlsConfig::from_pem_file(&cert, &key).await?;
    let reload = config.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(600));
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = reload.reload_from_pem_file(&cert, &key).await {
                tracing::warn!(error = %e, "reloading the TLS certificate failed");
            }
        }
    });
    let handle = axum_server::Handle::new();
    let draining = handle.clone();
    let shutting = Arc::clone(&server);
    tokio::spawn(async move {
        stop.await;
        shutting.begin_shutdown();
        draining.graceful_shutdown(Some(Duration::from_secs(25)));
    });
    let app = router(server);
    axum_server::bind_rustls(addr, config)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;
    Ok(())
}
