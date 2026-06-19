use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub pid_file: Option<String>,
    pub shutdown_timeout_secs: Option<u64>,
    pub log_level: Option<String>,
}

fn default_listen_addr() -> String { "127.0.0.1".to_string() }
fn default_port() -> u16 { 4040 }

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
