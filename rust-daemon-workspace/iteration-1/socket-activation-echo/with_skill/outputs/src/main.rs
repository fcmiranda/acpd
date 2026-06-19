use anyhow::Result;

mod config;
mod daemon;
mod logging;
mod pid;
mod signals;
mod socket_activation;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse --config flag or use default path
    let config_path = parse_config_path();
    let config = config::Config::load_or_default(&config_path);

    // Initialize logging
    let log_level = config.log_level.as_deref().unwrap_or("info");
    logging::init_logging(log_level);

    tracing::info!("starting echo-daemon v{}", env!("CARGO_PKG_VERSION"));

    // Create PID file if configured
    let _pid = if let Some(ref path) = config.pid_file {
        Some(pid::PidFile::create(path)?)
    } else {
        None
    };

    // Run the daemon (handles signals, shutdown, sd_notify internally)
    daemon::run(config).await?;

    tracing::info!("echo-daemon stopped cleanly");
    Ok(())
}

/// Parse the --config <path> argument from the command line.
fn parse_config_path() -> String {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--config" {
            if let Some(path) = args.get(i + 1) {
                return path.clone();
            }
        }
    }
    "/etc/echo-daemon/config.toml".to_string()
}
