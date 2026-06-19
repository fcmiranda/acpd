// signals.rs — Async signal handling for logwatchd.
//
// Follows the skill pattern: use tokio::signal::unix for async signal handling,
// broadcast shutdown intent via a watch channel, and handle SIGHUP separately
// for config reload (SIGHUP should NOT trigger shutdown).

use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{mpsc, watch};

/// Events that can be triggered by Unix signals.
#[derive(Debug, Clone)]
pub enum SignalEvent {
    /// SIGTERM or SIGINT received — initiate graceful shutdown.
    Shutdown,
    /// SIGHUP received — reload configuration.
    Reload,
}

/// Listen for Unix signals and dispatch events.
///
/// - SIGTERM / SIGINT → sends `true` on `shutdown_tx` to broadcast shutdown intent.
/// - SIGHUP → sends `Reload` event on `reload_tx` for config hot-reload.
///
/// This function runs in a loop so SIGHUP can be received multiple times
/// without stopping the signal listener. Only SIGTERM/SIGINT break the loop.
pub async fn signal_listener(
    shutdown_tx: watch::Sender<bool>,
    reload_tx: mpsc::Sender<SignalEvent>,
) {
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
                tracing::info!("received SIGHUP, triggering configuration reload");
                if let Err(e) = reload_tx.send(SignalEvent::Reload).await {
                    tracing::error!("failed to send reload event: {e}");
                }
                // Do NOT return — keep listening for more signals
            }
        }
    }
}
