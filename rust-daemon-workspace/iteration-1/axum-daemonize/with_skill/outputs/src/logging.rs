use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize the tracing/logging subsystem.
///
/// Attempts to connect to journald first (preferred when running under
/// systemd). If journald is not available (e.g. during local development),
/// falls back to structured stdout logging.
pub fn init_logging(log_level: &str) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    // Try journald first — works seamlessly with `journalctl -u api-gateway`
    if let Ok(journald_layer) = tracing_journald::layer() {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(journald_layer)
            .init();
        return;
    }

    // Fallback: pretty stdout logging for local development
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
