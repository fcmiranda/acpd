use tokio::signal::unix::{signal, SignalKind};

/// Listens for termination signals and broadcasts shutdown intent
/// via the provided watch channel.
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
                let _ = shutdown_tx.send(true);
                return;
            }
            _ = sigint.recv() => {
                tracing::info!("received SIGINT, initiating graceful shutdown");
                let _ = shutdown_tx.send(true);
                return;
            }
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP, reload not implemented — ignoring");
                // SIGHUP is conventionally used for config reload, not shutdown.
                // Continue listening for more signals.
            }
        }
    }
}
