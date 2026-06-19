use crate::config::Config;
use crate::signals;
use crate::socket_activation;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Run the echo daemon — accept connections, echo lines, handle shutdown.
pub async fn run(config: Config) -> Result<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn signal listener
    tokio::spawn(signals::signal_listener(shutdown_tx));

    // Obtain the TCP listener: socket activation first, fallback to manual bind
    let listener = if let Some(systemd_listener) = socket_activation::get_systemd_socket() {
        tracing::info!("using systemd socket activation");
        TcpListener::from_std(systemd_listener)?
    } else {
        let addr = format!("{}:{}", config.listen_addr, config.port);
        tracing::info!("binding to {addr}");
        TcpListener::bind(&addr).await?
    };

    // Notify systemd we're ready
    sd_notify::notify(false, &[sd_notify::NotifyState::Ready])?;
    tracing::info!("echo-daemon is ready and accepting connections");

    // Spawn the watchdog ticker
    let watchdog_shutdown_rx = shutdown_rx.clone();
    tokio::spawn(watchdog_loop(watchdog_shutdown_rx));

    // Accept loop
    let accept_shutdown_rx = shutdown_rx.clone();
    accept_loop(listener, accept_shutdown_rx).await;

    // Give in-flight connections time to drain
    let drain_timeout = tokio::time::Duration::from_secs(
        config.shutdown_timeout_secs.unwrap_or(30),
    );
    tracing::info!(
        "draining in-flight connections (timeout: {}s)",
        drain_timeout.as_secs()
    );
    tokio::time::sleep(std::cmp::min(
        drain_timeout,
        tokio::time::Duration::from_secs(2),
    ))
    .await;

    // Notify systemd we're stopping
    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;
    tracing::info!("echo-daemon shutdown complete");

    Ok(())
}

/// Accept loop — runs until shutdown is signalled.
async fn accept_loop(listener: TcpListener, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, peer_addr)) => {
                        tracing::info!(%peer_addr, "accepted connection");
                        let conn_shutdown_rx = shutdown_rx.clone();
                        tokio::spawn(handle_connection(stream, peer_addr, conn_shutdown_rx));
                    }
                    Err(e) => {
                        tracing::error!("failed to accept connection: {e}");
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::info!("stopping accept loop");
                break;
            }
        }
    }
}

/// Handle a single client connection — read lines and echo them back.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        tokio::select! {
            result = buf_reader.read_line(&mut line) => {
                match result {
                    Ok(0) => {
                        // EOF — client disconnected
                        tracing::info!(%peer_addr, "client disconnected");
                        return;
                    }
                    Ok(_n) => {
                        if let Err(e) = writer.write_all(line.as_bytes()).await {
                            tracing::error!(%peer_addr, "write error: {e}");
                            return;
                        }
                    }
                    Err(e) => {
                        tracing::error!(%peer_addr, "read error: {e}");
                        return;
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::info!(%peer_addr, "closing connection due to shutdown");
                let _ = writer.write_all(b"server shutting down\n").await;
                return;
            }
        }
    }
}

/// Periodically ping the systemd watchdog.
async fn watchdog_loop(mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
    // Ping every 15s — must be less than WatchdogSec/2 (30s / 2 = 15s)
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => break,
            _ = interval.tick() => {
                sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog])
                    .ok(); // Don't crash if not running under systemd
            }
        }
    }
}
