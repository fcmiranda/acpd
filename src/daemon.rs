use crate::config::Config;
use crate::health::health_router;
use crate::api::api_router;
use crate::adapters::{OutputAdapter, TmuxAdapter};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;

pub async fn run(config: Config) -> anyhow::Result<()> {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    let signal_handle = tokio::spawn(crate::signals::signal_listener(shutdown_tx));

    tracing::info!("Daemon binding to {}:{}", config.listen_addr, config.port);
    let listener = TcpListener::bind(format!("{}:{}", config.listen_addr, config.port)).await?;
    
    let start_time = Arc::new(Instant::now());
    
    let adapters: Arc<Vec<Box<dyn OutputAdapter>>> = Arc::new(vec![
        Box::new(TmuxAdapter),
    ]);
    
    let app = axum::Router::new()
        .merge(health_router(start_time))
        .merge(api_router(adapters));

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await
    });

    sd_notify::notify(false, &[sd_notify::NotifyState::Ready])?;

    signal_handle.await?;

    match tokio::time::timeout(std::time::Duration::from_secs(config.shutdown_timeout_secs.unwrap_or(30)), server_handle).await {
        Ok(Ok(Ok(()))) => tracing::info!("axum server cleanly shutdown"),
        Ok(Ok(Err(e))) => tracing::error!("axum server error: {}", e),
        Ok(Err(e)) => tracing::error!("server task panicked: {}", e),
        Err(_) => tracing::error!("server shutdown timed out"),
    }

    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;

    Ok(())
}
