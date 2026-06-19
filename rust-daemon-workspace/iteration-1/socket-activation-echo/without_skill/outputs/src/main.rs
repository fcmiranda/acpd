//! Socket-activated TCP echo daemon.
//!
//! When launched by systemd with socket activation, this daemon accepts the
//! listener file descriptor passed via the LISTEN_FDS protocol (fd 3).
//! When launched standalone, it falls back to binding its own socket on
//! 0.0.0.0:9090.

use std::env;
use std::os::unix::io::{FromRawFd, RawFd};
use std::process;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

/// The first file descriptor passed by systemd socket activation.
const SD_LISTEN_FDS_START: RawFd = 3;

/// Default bind address when not socket-activated.
const DEFAULT_BIND_ADDR: &str = "0.0.0.0:9090";

/// Attempt to retrieve a `TcpListener` from systemd socket activation.
///
/// systemd sets two environment variables:
/// - `LISTEN_PID`: must match the current process PID
/// - `LISTEN_FDS`: the number of file descriptors passed (we expect exactly 1)
///
/// The first fd is always 3 (`SD_LISTEN_FDS_START`).
fn try_systemd_socket() -> Option<std::net::TcpListener> {
    let listen_pid: u32 = env::var("LISTEN_PID").ok()?.parse().ok()?;
    let listen_fds: u32 = env::var("LISTEN_FDS").ok()?.parse().ok()?;

    let my_pid = process::id();

    if listen_pid != my_pid {
        warn!(
            listen_pid,
            my_pid, "LISTEN_PID does not match current PID, ignoring"
        );
        return None;
    }

    if listen_fds == 0 {
        warn!("LISTEN_FDS is 0, no sockets passed");
        return None;
    }

    if listen_fds > 1 {
        warn!(listen_fds, "Multiple fds passed, using only the first one");
    }

    // SAFETY: We trust systemd to pass a valid, open socket fd at position 3.
    // We also set it to non-blocking since tokio requires that.
    let std_listener = unsafe { std::net::TcpListener::from_raw_fd(SD_LISTEN_FDS_START) };
    std_listener.set_nonblocking(true).ok()?;

    info!("Acquired socket from systemd (fd {})", SD_LISTEN_FDS_START);
    Some(std_listener)
}

/// Handle a single TCP client connection: read lines and echo them back.
async fn handle_client(stream: TcpStream, peer: std::net::SocketAddr) {
    info!(%peer, "New client connected");

    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match buf_reader.read_line(&mut line).await {
            Ok(0) => {
                info!(%peer, "Client disconnected");
                break;
            }
            Ok(_n) => {
                if let Err(e) = writer.write_all(line.as_bytes()).await {
                    error!(%peer, error = %e, "Failed to write to client");
                    break;
                }
            }
            Err(e) => {
                error!(%peer, error = %e, "Failed to read from client");
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // Try systemd socket activation first, fall back to self-binding.
    let listener = if let Some(std_listener) = try_systemd_socket() {
        TcpListener::from_std(std_listener)?
    } else {
        info!(
            addr = DEFAULT_BIND_ADDR,
            "No systemd socket detected, binding our own listener"
        );
        TcpListener::bind(DEFAULT_BIND_ADDR).await?
    };

    info!(
        addr = %listener.local_addr()?,
        "Echo server listening"
    );

    // Accept loop.
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tokio::spawn(handle_client(stream, peer));
            }
            Err(e) => {
                error!(error = %e, "Failed to accept connection");
            }
        }
    }
}
