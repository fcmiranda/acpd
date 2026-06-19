use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod daemon;
mod health;
mod pid;
mod signals;
mod api;
mod state;
mod adapters;

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/etc/acpd/config.toml".to_string());
    
    let config = config::Config::load(&config_path).unwrap_or_else(|_| {
        config::Config {
            listen_addr: "127.0.0.1".to_string(),
            port: 4040,
            pid_file: None,
            shutdown_timeout_secs: Some(30),
            log_level: Some("info".to_string()),
        }
    });

    let log_level = config.log_level.as_deref().unwrap_or("info");
    init_logging(log_level);

    tracing::info!("starting acpd v{}", env!("CARGO_PKG_VERSION"));

    let _pid = if let Some(ref path) = config.pid_file {
        Some(pid::PidFile::create(path)?)
    } else {
        None
    };

    daemon::run(config).await?;

    tracing::info!("acpd stopped cleanly");
    Ok(())
}

pub fn init_logging(log_level: &str) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    if let Ok(journald_layer) = tracing_journald::layer() {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(journald_layer)
            .init();
        return;
    }

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true).with_thread_ids(true))
        .init();
}
