use serde::Deserialize;

/// Top-level configuration for the api-gateway daemon.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Address to bind the HTTP server to.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// Port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Optional path for the PID file.
    pub pid_file: Option<String>,

    /// Seconds to wait for in-flight requests to drain during shutdown.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,

    /// Minimum log level (trace, debug, info, warn, error).
    pub log_level: Option<String>,

    /// Interval in seconds for systemd watchdog pings.
    #[serde(default = "default_watchdog_interval")]
    pub watchdog_interval_secs: u64,
}

fn default_listen_addr() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_shutdown_timeout() -> u64 {
    30
}

fn default_watchdog_interval() -> u64 {
    10
}

impl Config {
    /// Load configuration from a TOML file, then apply environment variable
    /// overrides (API_GATEWAY_PORT, API_GATEWAY_LISTEN_ADDR, etc.).
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;

        // Environment variable overrides
        if let Ok(val) = std::env::var("API_GATEWAY_LISTEN_ADDR") {
            config.listen_addr = val;
        }
        if let Ok(val) = std::env::var("API_GATEWAY_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                config.port = port;
            }
        }
        if let Ok(val) = std::env::var("API_GATEWAY_LOG_LEVEL") {
            config.log_level = Some(val);
        }

        Ok(config)
    }
}
