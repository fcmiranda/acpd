// config.rs — Configuration loading, validation, and hot-reload support for logwatchd.
//
// Assumptions:
// - Config file is TOML at /etc/logwatchd/config.toml (overridable via --config flag)
// - Sensible defaults are provided for all fields so a minimal config works
// - Config is loaded into an ArcSwap for lock-free concurrent reads during hot-reload

use anyhow::{Context, Result};
use serde::Deserialize;
use std::sync::Arc;

/// Top-level daemon configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Directory to watch for log files.
    #[serde(default = "default_watch_dir")]
    pub watch_dir: String,

    /// Glob patterns for matching log file names.
    #[serde(default = "default_file_patterns")]
    pub file_patterns: Vec<String>,

    /// Regex patterns that indicate an error line.
    #[serde(default = "default_error_patterns")]
    pub error_patterns: Vec<String>,

    /// Interval in seconds between file scans.
    #[serde(default = "default_scan_interval")]
    pub scan_interval_secs: u64,

    /// Maximum notifications per minute (rate limiting).
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_minute: u32,

    /// Notification display settings.
    #[serde(default)]
    pub notification: NotificationConfig,

    /// Daemon runtime settings.
    #[serde(default)]
    pub daemon: DaemonConfig,
}

/// Settings for desktop notification display.
#[derive(Debug, Clone, Deserialize)]
pub struct NotificationConfig {
    /// Application name shown in the notification bubble.
    #[serde(default = "default_app_name")]
    pub app_name: String,

    /// Urgency level: "low", "normal", or "critical".
    #[serde(default = "default_urgency")]
    pub urgency: String,

    /// Notification timeout in seconds. 0 = use system default.
    #[serde(default)]
    pub timeout_secs: u32,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            app_name: default_app_name(),
            urgency: default_urgency(),
            timeout_secs: 0,
        }
    }
}

/// Daemon runtime and lifecycle settings.
#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    /// Path to PID file. Empty string disables PID file creation.
    #[serde(default = "default_pid_file")]
    pub pid_file: String,

    /// Graceful shutdown timeout in seconds.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,

    /// Log level filter string (e.g. "info", "debug", "warn").
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            pid_file: default_pid_file(),
            shutdown_timeout_secs: default_shutdown_timeout(),
            log_level: default_log_level(),
        }
    }
}

// --- Default value functions ---

fn default_watch_dir() -> String {
    "/var/log".to_string()
}

fn default_file_patterns() -> Vec<String> {
    vec![
        "*.log".to_string(),
        "*.err".to_string(),
        "syslog".to_string(),
        "messages".to_string(),
    ]
}

fn default_error_patterns() -> Vec<String> {
    vec![
        r"(?i)\bERROR\b".to_string(),
        r"(?i)\bFATAL\b".to_string(),
        r"(?i)\bCRITICAL\b".to_string(),
        r"(?i)\bPANIC\b".to_string(),
    ]
}

fn default_scan_interval() -> u64 {
    5
}

fn default_rate_limit() -> u32 {
    10
}

fn default_app_name() -> String {
    "logwatchd".to_string()
}

fn default_urgency() -> String {
    "critical".to_string()
}

fn default_pid_file() -> String {
    "/run/logwatchd/logwatchd.pid".to_string()
}

fn default_shutdown_timeout() -> u64 {
    30
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    /// Load configuration from a TOML file at the given path.
    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {path}"))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config file: {path}"))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values after loading.
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.watch_dir.is_empty(),
            "watch_dir must not be empty"
        );
        anyhow::ensure!(
            !self.error_patterns.is_empty(),
            "error_patterns must contain at least one pattern"
        );
        anyhow::ensure!(
            self.scan_interval_secs > 0,
            "scan_interval_secs must be greater than 0"
        );
        // Validate that all error patterns are valid regexes
        for pattern in &self.error_patterns {
            regex::Regex::new(pattern)
                .with_context(|| format!("invalid error pattern regex: {pattern}"))?;
        }
        // Validate urgency
        match self.notification.urgency.as_str() {
            "low" | "normal" | "critical" => {}
            other => anyhow::bail!("invalid notification urgency: {other} (expected low, normal, or critical)"),
        }
        Ok(())
    }
}

/// Shared config handle that supports lock-free reads and atomic swaps for hot-reload.
/// Wraps Arc so cloning is cheap across tasks.
pub type SharedConfig = Arc<arc_swap::ArcSwap<Config>>;

/// Create a new shared config handle from an initial Config.
pub fn shared_config(config: Config) -> SharedConfig {
    Arc::new(arc_swap::ArcSwap::from_pointee(config))
}
