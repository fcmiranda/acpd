use std::fs;
use std::io::Write;
use std::process;

/// Manages a PID file for the daemon process.
///
/// On creation, checks for an existing PID file and validates whether
/// the referenced process is still alive. If the old process is gone,
/// the stale PID file is removed. The file is automatically deleted
/// when this struct is dropped.
pub struct PidFile {
    path: String,
}

impl PidFile {
    /// Create a new PID file at the given path.
    ///
    /// Returns an error if another instance is already running (as
    /// indicated by a live process matching the PID in the existing file).
    pub fn create(path: &str) -> anyhow::Result<Self> {
        // Check for a stale PID file from a previous run
        if let Ok(existing) = fs::read_to_string(path) {
            if let Ok(pid) = existing.trim().parse::<i32>() {
                // Signal 0 checks if the process exists without actually signalling it
                if nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    None,
                )
                .is_ok()
                {
                    anyhow::bail!(
                        "api-gateway already running with PID {pid} (PID file: {path})"
                    );
                }
                tracing::warn!("removing stale PID file for PID {pid}");
            }
        }

        // Ensure the parent directory exists
        if let Some(parent) = std::path::Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(path)?;
        write!(file, "{}", process::id())?;
        tracing::info!(pid = process::id(), path = path, "PID file created");

        Ok(PidFile {
            path: path.to_string(),
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path) {
            tracing::error!("failed to remove PID file {}: {e}", self.path);
        } else {
            tracing::info!(path = %self.path, "PID file removed");
        }
    }
}
