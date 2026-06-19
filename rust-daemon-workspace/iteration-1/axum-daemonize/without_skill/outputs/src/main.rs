//! api-gateway: A daemonized axum HTTP server with systemd integration.
//!
//! Features:
//! - systemd Type=notify with sd_notify readiness signaling
//! - Watchdog keepalive pings
//! - Health check endpoint at /health
//! - PID file management
//! - Graceful shutdown draining in-flight HTTP requests
//! - TOML-based configuration
//! - Structured logging via tracing (journald + stdout)

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::watch;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Top-level configuration loaded from a TOML file.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_server")]
    pub server: ServerConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Seconds to wait for in-flight requests during shutdown.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DaemonConfig {
    #[serde(default = "default_pid_file")]
    pub pid_file: PathBuf,
    /// Enable systemd watchdog keepalive (auto-detected from WATCHDOG_USEC).
    #[serde(default = "default_true")]
    pub watchdog_enabled: bool,
}

fn default_server() -> ServerConfig {
    ServerConfig {
        host: default_host(),
        port: default_port(),
        shutdown_timeout_secs: default_shutdown_timeout(),
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            pid_file: default_pid_file(),
            watchdog_enabled: true,
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    3000
}
fn default_shutdown_timeout() -> u64 {
    30
}
fn default_pid_file() -> PathBuf {
    PathBuf::from("/run/api-gateway/api-gateway.pid")
}
fn default_true() -> bool {
    true
}

impl Config {
    /// Load configuration from a TOML file. Falls back to defaults if the file
    /// doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file: {}", path.display()))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {}", path.display()))
        } else {
            warn!(
                path = %path.display(),
                "Config file not found, using defaults"
            );
            Ok(Config {
                server: default_server(),
                daemon: DaemonConfig::default(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Application state shared across handlers
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    /// Indicates whether the server is healthy and ready to serve traffic.
    healthy: Arc<AtomicBool>,
    /// Timestamp when the server started (for uptime reporting).
    started_at: Instant,
}

impl AppState {
    fn new() -> Self {
        Self {
            healthy: Arc::new(AtomicBool::new(true)),
            started_at: Instant::now(),
        }
    }

    fn mark_unhealthy(&self) {
        self.healthy.store(false, Ordering::SeqCst);
    }

    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Health check types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_secs: u64,
    pid: u32,
}

// ---------------------------------------------------------------------------
// HTTP Handlers
// ---------------------------------------------------------------------------

/// GET /health — returns 200 when healthy, 503 when shutting down.
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    if state.is_healthy() {
        let body = HealthResponse {
            status: "healthy",
            uptime_secs: state.started_at.elapsed().as_secs(),
            pid: process::id(),
        };
        (StatusCode::OK, Json(body))
    } else {
        let body = HealthResponse {
            status: "shutting_down",
            uptime_secs: state.started_at.elapsed().as_secs(),
            pid: process::id(),
        };
        (StatusCode::SERVICE_UNAVAILABLE, Json(body))
    }
}

/// GET / — simple root handler (placeholder for actual API routes).
async fn root() -> &'static str {
    "api-gateway is running"
}

// ---------------------------------------------------------------------------
// PID file management
// ---------------------------------------------------------------------------

struct PidFile {
    path: PathBuf,
}

impl PidFile {
    /// Write the current process PID to the file. Creates parent directories
    /// if needed.
    fn create(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create PID file directory: {}", parent.display())
            })?;
        }
        let pid = process::id();
        fs::write(&path, pid.to_string())
            .with_context(|| format!("Failed to write PID file: {}", path.display()))?;
        info!(pid, path = %path.display(), "PID file created");
        Ok(Self { path })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path) {
            warn!(
                error = %e,
                path = %self.path.display(),
                "Failed to remove PID file on shutdown"
            );
        } else {
            info!(path = %self.path.display(), "PID file removed");
        }
    }
}

// ---------------------------------------------------------------------------
// Systemd helpers
// ---------------------------------------------------------------------------

/// Notify systemd that the service is ready (READY=1).
fn sd_notify_ready() {
    if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        warn!(error = %e, "Failed to send READY=1 to systemd");
    } else {
        info!("Notified systemd: READY=1");
    }
}

/// Notify systemd that the service is stopping.
fn sd_notify_stopping() {
    if let Err(e) = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]) {
        warn!(error = %e, "Failed to send STOPPING=1 to systemd");
    } else {
        info!("Notified systemd: STOPPING=1");
    }
}

/// Notify systemd with a status message.
fn sd_notify_status(msg: &str) {
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Status(msg.into())]);
}

/// Send a watchdog keepalive ping to systemd.
fn sd_notify_watchdog() {
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]);
}

/// Determine the watchdog interval from the WATCHDOG_USEC environment variable.
/// Returns `None` if watchdog is not configured.
fn watchdog_interval() -> Option<Duration> {
    std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|val| val.parse::<u64>().ok())
        .map(|usec| {
            // Ping at half the watchdog interval to be safe.
            Duration::from_micros(usec / 2)
        })
}

/// Spawn a background task that pings the watchdog at the required interval.
fn spawn_watchdog_task(mut shutdown_rx: watch::Receiver<bool>) {
    if let Some(interval) = watchdog_interval() {
        info!(
            interval_ms = interval.as_millis() as u64,
            "Starting systemd watchdog keepalive task"
        );
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        sd_notify_watchdog();
                    }
                    _ = shutdown_rx.changed() => {
                        info!("Watchdog task stopping");
                        return;
                    }
                }
            }
        });
    } else {
        info!("Watchdog not configured (WATCHDOG_USEC not set)");
    }
}

// ---------------------------------------------------------------------------
// Signal handling & graceful shutdown
// ---------------------------------------------------------------------------

/// Wait for SIGTERM or SIGINT, then initiate graceful shutdown.
async fn shutdown_signal(
    shutdown_tx: watch::Sender<bool>,
    state: AppState,
    shutdown_timeout: Duration,
) {
    let sigterm = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler")
            .recv()
            .await;
    };

    let sigint = async {
        signal::ctrl_c()
            .await
            .expect("failed to register SIGINT handler");
    };

    tokio::select! {
        _ = sigterm => info!("Received SIGTERM"),
        _ = sigint => info!("Received SIGINT"),
    }

    info!("Initiating graceful shutdown...");

    // 1. Tell systemd we're stopping.
    sd_notify_stopping();
    sd_notify_status("Shutting down, draining in-flight requests...");

    // 2. Mark health endpoint as unhealthy so load balancers stop sending traffic.
    state.mark_unhealthy();

    // 3. Signal all background tasks (watchdog, etc.) to stop.
    let _ = shutdown_tx.send(true);

    info!(
        timeout_secs = shutdown_timeout.as_secs(),
        "Waiting for in-flight requests to drain"
    );
}

/// Wait for a SIGHUP to trigger configuration reload (placeholder).
fn spawn_sighup_handler() {
    tokio::spawn(async {
        let mut stream = signal::unix::signal(signal::unix::SignalKind::hangup())
            .expect("failed to register SIGHUP handler");
        loop {
            stream.recv().await;
            info!("Received SIGHUP — configuration reload triggered (not yet implemented)");
            // In a real daemon you'd reload config here.
        }
    });
}

// ---------------------------------------------------------------------------
// Logging / tracing initialization
// ---------------------------------------------------------------------------

fn init_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=debug"));

    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    // Attempt to add journald layer; fall back to stdout-only if unavailable.
    match tracing_journald::layer() {
        Ok(journald_layer) => {
            registry.with(journald_layer).init();
        }
        Err(_) => {
            registry.init();
        }
    }
}

// ---------------------------------------------------------------------------
// Router construction
// ---------------------------------------------------------------------------

fn build_router(state: AppState) -> Router {
    use tower_http::trace::TraceLayer;

    Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = process::id(),
        "Starting api-gateway"
    );

    // Load configuration.
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/api-gateway/config.toml"));

    let config = Config::load(&config_path)?;
    info!(?config, "Configuration loaded");

    // Create PID file.
    let _pid_file = PidFile::create(config.daemon.pid_file.clone())?;

    // Build application state and router.
    let state = AppState::new();
    let app = build_router(state.clone());

    // Bind the TCP listener.
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .context("Invalid listen address")?;

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind to {addr}"))?;

    info!(%addr, "Listening");

    // Create a shutdown signal channel.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start watchdog keepalive.
    if config.daemon.watchdog_enabled {
        spawn_watchdog_task(shutdown_rx.clone());
    }

    // Start SIGHUP handler for config reload.
    spawn_sighup_handler();

    // Notify systemd that we are ready.
    sd_notify_ready();
    sd_notify_status("Listening and serving requests");

    let shutdown_timeout = Duration::from_secs(config.server.shutdown_timeout_secs);
    let shutdown_state = state.clone();

    // Serve with graceful shutdown.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_tx, shutdown_state, shutdown_timeout))
        .await
        .context("Server error")?;

    info!("All connections drained. Shutting down cleanly.");
    sd_notify_status("Stopped");

    // _pid_file is dropped here, removing the PID file.
    Ok(())
}
