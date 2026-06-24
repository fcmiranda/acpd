use crate::adapters::{TmuxAdapter, WaybarAdapter};
use crate::api::api_router;
use crate::config::Config;
use crate::health::health_router;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;

pub async fn run(config: Config) -> anyhow::Result<()> {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    let signal_handle = tokio::spawn(crate::signals::signal_listener(shutdown_tx));

    tracing::info!("Daemon binding to {}:{}", config.listen_addr, config.port);
    let listener = TcpListener::bind(format!("{}:{}", config.listen_addr, config.port)).await?;

    let start_time = Arc::new(Instant::now());

    let active_spinner_name = config
        .theme
        .as_ref()
        .map(|t| t.active_spinner.clone())
        .unwrap_or_else(|| "arc".to_string());
        
    let active_spinner = config
        .spinners
        .as_ref()
        .and_then(|spinners| spinners.get(&active_spinner_name).cloned());
    tokio::spawn(async move {
        let _ = tokio::process::Command::new("tmux")
            .args(["set", "-g", "@ai_agent_spinner", &active_spinner_name])
            .output()
            .await;
    });

    let adapters: crate::api::ApiState = crate::api::ApiState {
        adapters: Arc::new(vec![
            Box::new(TmuxAdapter::new(config.theme.clone(), active_spinner)),
            Box::new(WaybarAdapter::new(config.theme.clone())),
        ]),
        pending_idles: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        idle_debounce_ms: config.idle_debounce_ms.unwrap_or(650),
    };

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

    match tokio::time::timeout(
        std::time::Duration::from_secs(config.shutdown_timeout_secs.unwrap_or(30)),
        server_handle,
    )
    .await
    {
        Ok(Ok(Ok(()))) => tracing::info!("axum server cleanly shutdown"),
        Ok(Ok(Err(e))) => tracing::error!("axum server error: {}", e),
        Ok(Err(e)) => tracing::error!("server task panicked: {}", e),
        Err(_) => tracing::error!("server shutdown timed out"),
    }

    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;

    Ok(())
}
