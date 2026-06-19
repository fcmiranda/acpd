use std::fs;
use std::io::Write;
use std::process;

/// Manages a PID file for the daemon process.
/// Automatically removes the PID file when dropped.
pub struct PidFile {
    path: String,
}

impl PidFile {
    pub fn create(path: &str) -> anyhow::Result<Self> {
        // Check for stale PID file
        if let Ok(existing) = fs::read_to_string(path) {
            if let Ok(pid) = existing.trim().parse::<i32>() {
                // Check if process is still running
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
                // Stale PID file, remove it
                tracing::warn!("removing stale PID file for PID {pid}");
            }
        }

        let mut file = fs::File::create(path)?;
        write!(file, "{}", process::id())?;
        Ok(PidFile {
            path: path.to_string(),
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path) {
            tracing::error!("failed to remove PID file {}: {e}", self.path);
        }
    }
}
