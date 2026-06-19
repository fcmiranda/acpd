// pid.rs — PID file management for logwatchd.
//
// Creates a PID file on startup, checks for stale PID files from previous
// crashes, and automatically removes the PID file on drop (even on unclean
// shutdown, as long as we're not SIGKILL'd).

use anyhow::Result;
use std::fs;
use std::io::Write;
use std::process;

/// RAII guard for a PID file. The file is removed when this value is dropped.
pub struct PidFile {
    path: String,
}

impl PidFile {
    /// Create a new PID file at the given path.
    ///
    /// Returns an error if another instance is already running (detected via
    /// the existing PID file pointing to a live process).
    pub fn create(path: &str) -> Result<Self> {
        // Check for a stale PID file left behind by a previous crash
        if let Ok(existing) = fs::read_to_string(path) {
            if let Ok(pid) = existing.trim().parse::<i32>() {
                // kill(pid, 0) checks if the process exists without sending a signal
                if nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    None,
                )
                .is_ok()
                {
                    anyhow::bail!(
                        "daemon already running with PID {pid} (PID file: {path})"
                    );
                }
                // Process is gone — stale PID file
                tracing::warn!(pid, path, "removing stale PID file");
            }
        }

        // Ensure the parent directory exists
        if let Some(parent) = std::path::Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(path)?;
        write!(file, "{}", process::id())?;
        tracing::debug!(pid = process::id(), path, "created PID file");

        Ok(PidFile {
            path: path.to_string(),
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path) {
            // Don't panic in Drop — just log the error
            tracing::error!(path = %self.path, error = %e, "failed to remove PID file");
        } else {
            tracing::debug!(path = %self.path, "removed PID file");
        }
    }
}
