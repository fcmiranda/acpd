use crate::config::Config;
use crate::health;
use crate::signals;
use axum::Router;
use std::time::Instant;
use tower_http::trace::TraceLayer;

/// Daemon entry point: starts the HTTP server, notifies systemd of readiness,
/// runs the watchdog ping loop, and handles graceful shutdown with request draining.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let start_time = Instant::now();

    // Build the combined application router
    let app = build_router(start_time);

    // Bind TCP listener
    let bind_addr = format!("{}:{}", config.listen_addr, config.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!(addr = %bind_addr, "HTTP server listening");

    // Shutdown coordination: a watch channel broadcasts shutdown intent
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn the OS signal listener (SIGTERM, SIGINT, SIGHUP)
    let signal_handle = tokio::spawn(signals::signal_listener(shutdown_tx));

    // Spawn the systemd watchdog ping loop
    let watchdog_interval = config.watchdog_interval_secs;
    let watchdog_rx = shutdown_rx.clone();
    let watchdog_handle = tokio::spawn(watchdog_loop(watchdog_interval, watchdog_rx));

    // Notify systemd that we are ready to serve traffic.
    // This is critical for Type=notify — systemd won't consider us
    // "started" until it receives this.
    sd_notify::notify(false, &[sd_notify::NotifyState::Ready])?;
    tracing::info!("notified systemd: READY");

    // Serve HTTP with axum's graceful shutdown support.
    // axum::serve().with_graceful_shutdown() stops accepting new connections
    // when the future resolves, and then waits for in-flight requests to
    // complete — exactly the draining behavior we need.
    let shutdown_signal = {
        let mut rx = shutdown_rx.clone();
        async move {
            // Wait until the watch value becomes `true`
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
    };

    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal);

    // Run the server. It will stop accepting connections when the shutdown
    // signal fires, then drain in-flight requests.
    let drain_timeout =
        tokio::time::Duration::from_secs(config.shutdown_timeout_secs);

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "HTTP server error");
            }
        }
        // If the signal fires, the graceful shutdown future will resolve
        // the server above. This branch is a safety net for the drain timeout.
        _ = async {
            // Wait for shutdown signal first
            let mut rx = shutdown_rx.clone();
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
            // Then apply the drain timeout
            tokio::time::sleep(drain_timeout).await;
            tracing::warn!(
                timeout_secs = config.shutdown_timeout_secs,
                "shutdown drain timeout exceeded, forcing exit"
            );
        } => {}
    }

    // Clean up the watchdog task
    watchdog_handle.abort();
    let _ = watchdog_handle.await;

    // Wait for the signal handler to finish (it should already be done)
    let _ = signal_handle.await;

    // Notify systemd we're stopping
    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;
    tracing::info!("notified systemd: STOPPING");

    Ok(())
}

/// Build the full application router, merging the health endpoints
/// with the main API routes.
fn build_router(start_time: Instant) -> Router {
    // Main API routes (placeholder for your actual business logic)
    let api_routes = Router::new()
        .route("/", axum::routing::get(root_handler));

    // Merge health check routes and API routes, add tracing middleware
    Router::new()
        .merge(health::health_router(start_time))
        .merge(api_routes)
        .layer(TraceLayer::new_for_http())
}

/// Root handler — placeholder for actual API gateway logic.
async fn root_handler() -> &'static str {
    "api-gateway is running"
}

/// Periodically pings the systemd watchdog.
///
/// The ping interval MUST be less than `WatchdogSec / 2` in the unit file.
/// If the daemon stops pinging, systemd considers it hung and restarts it.
async fn watchdog_loop(
    interval_secs: u64,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(
        tokio::time::Duration::from_secs(interval_secs),
    );

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                tracing::debug!("watchdog loop stopping due to shutdown");
                break;
            }
            _ = interval.tick() => {
                // Ping systemd watchdog. .ok() because this is a no-op
                // when not running under systemd.
                sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]).ok();
                tracing::trace!("watchdog ping sent");
            }
        }
    }
}
