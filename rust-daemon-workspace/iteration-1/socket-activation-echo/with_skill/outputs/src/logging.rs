use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize structured logging.
/// Tries journald first (for systemd environments), falls back to stdout.
pub fn init_logging(log_level: &str) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    // Try journald first (works when running under systemd)
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
        .with(fmt::layer().with_target(true).with_thread_ids(true))
        .init();
}
