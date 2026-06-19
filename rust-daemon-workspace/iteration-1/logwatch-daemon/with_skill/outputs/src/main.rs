// main.rs — Entry point for logwatchd.
//
// logwatchd is a Rust daemon that monitors a directory for new log files and
// sends desktop notifications when error patterns are found. It runs as a
// systemd service with Type=notify integration.
//
// Design decisions and assumptions:
// - Config path defaults to /etc/logwatchd/config.toml, overridable via --config.
// - Logging initializes with journald when running under systemd, falls back to stdout.
// - PID file creation is optional (controlled by config).
// - The daemon uses tokio for async I/O, inotify for file watching, and D-Bus for
//   desktop notifications.
// - SIGHUP reloads config (file patterns, error patterns, rate limits).
// - SIGTERM/SIGINT trigger graceful shutdown with a configurable timeout.
// - The systemd watchdog is pinged every 25s (unit file sets WatchdogSec=60s).

use anyhow::Result;

mod config;
mod daemon;
mod notifier;
mod pid;
mod signals;
mod watcher;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments for --config flag
    let config_path = parse_config_path();

    // Load configuration
    let cfg = config::Config::load(&config_path)?;

    // Initialize logging (journald when available, stdout fallback)
    init_logging(&cfg.daemon.log_level);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %config_path,
        "starting logwatchd"
    );

    // Create PID file if configured
    let _pid_guard = if !cfg.daemon.pid_file.is_empty() {
        Some(pid::PidFile::create(&cfg.daemon.pid_file)?)
    } else {
        None
    };

    // Run the daemon — this blocks until shutdown is complete
    daemon::run(cfg, config_path).await?;

    tracing::info!("logwatchd stopped cleanly");
    Ok(())
}

/// Parse --config <path> from command-line arguments.
/// Falls back to /etc/logwatchd/config.toml if not provided.
fn parse_config_path() -> String {
    let args: Vec<String> = std::env::args().collect();

    for i in 0..args.len() {
        if args[i] == "--config" {
            if let Some(path) = args.get(i + 1) {
                return path.clone();
            }
        }
    }

    // Default config path
    "/etc/logwatchd/config.toml".to_string()
}

/// Initialize tracing-based structured logging.
///
/// Tries journald first (for systemd integration), falls back to
/// stdout with structured formatting for development.
fn init_logging(log_level: &str) {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    // Try journald first — works when running under systemd
    if let Ok(journald_layer) = tracing_journald::layer() {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(journald_layer)
            .init();
        return;
    }

    // Fall back to stdout with structured formatting
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(true)
                .with_file(true)
                .with_line_number(true),
        )
        .init();
}
