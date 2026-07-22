mod adapters;
mod api;
mod config;
mod daemon;
mod health;
mod pid;
mod signals;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize logging (tracing)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "acpd=info,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting ACP Daemon (acpd)...");

    // 2. Parse arguments and resolve config path
    let args: Vec<String> = std::env::args().collect();
    let mut config_path = None;
    for i in 0..args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_path = Some(args[i + 1].clone());
            break;
        }
    }

    let config_path = match config_path {
        Some(path) => path,
        None => {
            let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                format!("{}/.config", home)
            });
            let user_path = format!("{}/acpd/config.toml", xdg_config);
            let system_path = "/etc/acpd/config.toml";
            let dev_path = "config/default.toml";

            if std::path::Path::new(&user_path).is_file() {
                user_path
            } else if std::path::Path::new(system_path).is_file() {
                system_path.to_string()
            } else if std::path::Path::new(dev_path).is_file() {
                tracing::info!("System/User config not found, falling back to {}", dev_path);
                dev_path.to_string()
            } else {
                anyhow::bail!(
                    "No config file found. Provide one with --config <path>, or create {}",
                    user_path
                );
            }
        }
    };

    tracing::info!("Loading config from {}", config_path);
    let config = crate::config::Config::load(&config_path)?;

    // 3. Handle PID file if configured
    let _pid_file = if let Some(ref path) = config.pid_file {
        tracing::info!("Creating PID file at {}", path);
        Some(crate::pid::PidFile::create(path)?)
    } else {
        None
    };

    // 4. Run the daemon lifecycle
    crate::daemon::run(config).await?;

    Ok(())
}
