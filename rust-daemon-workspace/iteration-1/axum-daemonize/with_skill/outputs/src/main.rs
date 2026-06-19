use anyhow::Result;

mod config;
mod daemon;
mod health;
mod logging;
mod pid;
mod signals;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse --config <path> from CLI arguments.
    // Supports both `--config /path/to/file` and positional fallback.
    let config_path = parse_config_path();
    let config = config::Config::load(&config_path)?;

    // Initialize structured logging (journald under systemd, stdout otherwise)
    let log_level = config.log_level.as_deref().unwrap_or("info");
    logging::init_logging(log_level);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %config_path,
        "starting api-gateway"
    );

    // Create PID file if configured.
    // The _pid_file binding keeps the PidFile alive (and its Drop cleanup
    // registered) for the entire lifetime of main().
    let _pid_file = if let Some(ref path) = config.pid_file {
        Some(pid::PidFile::create(path)?)
    } else {
        None
    };

    // Run the daemon — this blocks until shutdown is complete
    daemon::run(config).await?;

    tracing::info!("api-gateway stopped cleanly");
    Ok(())
}

/// Parse the `--config <path>` CLI argument.
/// Falls back to `/etc/api-gateway/config.toml` if not provided.
fn parse_config_path() -> String {
    let args: Vec<String> = std::env::args().collect();

    for i in 0..args.len() {
        if args[i] == "--config" {
            if let Some(path) = args.get(i + 1) {
                return path.clone();
            }
        }
    }

    "/etc/api-gateway/config.toml".to_string()
}
