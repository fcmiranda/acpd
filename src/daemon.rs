use crate::config::Config;

pub async fn run(config: Config) -> anyhow::Result<()> {
    let (shutdown_tx, mut _shutdown_rx) = tokio::sync::watch::channel(false);

    let signal_handle = tokio::spawn(crate::signals::signal_listener(shutdown_tx));

    tracing::info!("Daemon running on {}:{}", config.listen_addr, config.port);
    
    // Notify systemd we're ready
    sd_notify::notify(false, &[sd_notify::NotifyState::Ready])?;

    // Wait for shutdown signal
    signal_handle.await?;

    // Notify systemd we're stopping
    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;

    Ok(())
}
