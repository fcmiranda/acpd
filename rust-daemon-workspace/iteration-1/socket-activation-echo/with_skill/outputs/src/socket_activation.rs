use std::net::TcpListener;
use std::os::unix::io::FromRawFd;

/// Retrieve socket-activated file descriptors from systemd.
/// Returns None if not running under socket activation.
pub fn get_systemd_socket() -> Option<TcpListener> {
    // SD_LISTEN_FDS_START is always 3
    const SD_LISTEN_FDS_START: i32 = 3;

    let listen_fds: i32 = std::env::var("LISTEN_FDS")
        .ok()?
        .parse()
        .ok()?;

    if listen_fds < 1 {
        return None;
    }

    // Safety: systemd guarantees this FD is valid when LISTEN_FDS is set
    let listener = unsafe { TcpListener::from_raw_fd(SD_LISTEN_FDS_START) };

    // Set non-blocking for tokio compatibility
    listener.set_nonblocking(true).ok()?;
    Some(listener)
}
