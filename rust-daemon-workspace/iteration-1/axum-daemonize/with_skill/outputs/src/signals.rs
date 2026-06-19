use tokio::signal::unix::{signal, SignalKind};

/// Listens for OS signals and broadcasts shutdown intent.
///
/// - SIGTERM / SIGINT → initiate graceful shutdown
/// - SIGHUP → log a reload message (placeholder for config reload)
///
/// This function blocks until a termination signal is received.
/// It will re-register after SIGHUP so that a subsequent SIGTERM
/// still triggers shutdown.
pub async fn signal_listener(shutdown_tx: tokio::sync::watch::Sender<bool>) {
    let mut sigterm = signal(SignalKind::terminate())
        .expect("failed to register SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt())
        .expect("failed to register SIGINT handler");
    let mut sighup = signal(SignalKind::hangup())
        .expect("failed to register SIGHUP handler");

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, initiating graceful shutdown");
                break;
            }
            _ = sigint.recv() => {
                tracing::info!("received SIGINT, initiating graceful shutdown");
                break;
            }
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP, configuration reload requested");
                // TODO: trigger actual config reload
                // Continue the loop so SIGTERM/SIGINT still works after SIGHUP
            }
        }
    }

    let _ = shutdown_tx.send(true);
}
