use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub pid_file: Option<String>,
    pub shutdown_timeout_secs: Option<u64>,
    pub log_level: Option<String>,
}

fn default_listen_addr() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    9090
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load from file if it exists, otherwise return defaults.
    pub fn load_or_default(path: &str) -> Self {
        match Self::load(path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("warning: could not load config from {path}: {e}, using defaults");
                Config {
                    listen_addr: default_listen_addr(),
                    port: default_port(),
                    pid_file: None,
                    shutdown_timeout_secs: Some(30),
                    log_level: Some("info".to_string()),
                }
            }
        }
    }
}
