// daemon.rs — Core daemon lifecycle orchestration for logwatchd.
//
// This module ties together all the components: signal handling, config reload,
// filesystem watching, and graceful shutdown. It follows the skill's recommended
// shutdown pattern:
//   1. Signal received → set shutdown flag via watch channel
//   2. Stop accepting new work (stop watcher)
//   3. Drain in-flight work with a timeout
//   4. Clean up resources (flush logs, remove PID file via RAII)
//   5. Notify systemd of stopping
//   6. Exit with code 0

use anyhow::Result;
use tokio::sync::{mpsc, watch};

use crate::config::{self, Config, SharedConfig};
use crate::signals::{self, SignalEvent};
use crate::watcher;

/// Run the daemon's main loop.
///
/// This function orchestrates all components and blocks until shutdown is complete.
/// It should be called from main() after config loading and logging initialization.
pub async fn run(config: Config, config_path: String) -> Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (reload_tx, mut reload_rx) = mpsc::channel::<SignalEvent>(4);

    // Wrap config in a shared handle for lock-free reads across tasks
    let shared_config: SharedConfig = config::shared_config(config);

    // Spawn the signal listener task
    let signal_handle = tokio::spawn(signals::signal_listener(
        shutdown_tx.clone(),
        reload_tx,
    ));

    // Spawn the filesystem watcher task
    let watcher_shutdown_rx = shutdown_rx.clone();
    let watcher_config = shared_config.clone();
    let watcher_handle = tokio::spawn(async move {
        if let Err(e) = watcher::run_watcher(watcher_config, watcher_shutdown_rx).await {
            tracing::error!(error = %e, "filesystem watcher failed");
        }
    });

    // Notify systemd that we're ready to serve
    sd_notify::notify(false, &[sd_notify::NotifyState::Ready])?;
    tracing::info!("logwatchd is ready and watching for log errors");

    // Spawn config reload handler
    let reload_config = shared_config.clone();
    let reload_handle = tokio::spawn(async move {
        while let Some(SignalEvent::Reload) = reload_rx.recv().await {
            tracing::info!(path = %config_path, "reloading configuration");
            match Config::load(&config_path) {
                Ok(new_config) => {
                    reload_config.store(std::sync::Arc::new(new_config));
                    tracing::info!("configuration reloaded successfully");
                    // Notify systemd of the reload
                    sd_notify::notify(
                        false,
                        &[sd_notify::NotifyState::Reloading],
                    )
                    .ok();
                    // Then signal ready again after reload
                    sd_notify::notify(
                        false,
                        &[sd_notify::NotifyState::Ready],
                    )
                    .ok();
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "failed to reload configuration, keeping current config"
                    );
                }
            }
        }
    });

    // Wait for the signal listener to complete (i.e., SIGTERM/SIGINT received)
    signal_handle.await?;

    // Begin graceful shutdown
    tracing::info!("starting graceful shutdown");

    // Give the watcher time to finish in-flight scans
    let drain_timeout = {
        let cfg = shared_config.load();
        tokio::time::Duration::from_secs(cfg.daemon.shutdown_timeout_secs)
    };

    match tokio::time::timeout(drain_timeout, watcher_handle).await {
        Ok(Ok(())) => tracing::info!("watcher shut down cleanly"),
        Ok(Err(e)) => tracing::error!(error = %e, "watcher task panicked"),
        Err(_) => tracing::warn!(
            timeout_secs = drain_timeout.as_secs(),
            "shutdown timed out waiting for watcher, forcing exit"
        ),
    }

    // The reload handler will exit when the channel is closed (signal_listener dropped reload_tx)
    reload_handle.abort();

    // Notify systemd we're stopping
    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;

    Ok(())
}
