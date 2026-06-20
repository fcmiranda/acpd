use tokio::signal::unix::{SignalKind, signal};

pub async fn signal_listener(shutdown_tx: tokio::sync::watch::Sender<bool>) {
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT handler");
    let mut sighup = signal(SignalKind::hangup()).expect("failed to register SIGHUP handler");

    tokio::select! {
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM, initiating graceful shutdown");
        }
        _ = sigint.recv() => {
            tracing::info!("received SIGINT, initiating graceful shutdown");
        }
        _ = sighup.recv() => {
            tracing::info!("received SIGHUP, reloading configuration not implemented yet");
            return;
        }
    }

    let _ = shutdown_tx.send(true);
}
