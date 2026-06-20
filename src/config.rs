use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Spinner {
    pub interval: u64,
    pub frames: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct Config {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub pid_file: Option<String>,
    pub shutdown_timeout_secs: Option<u64>,
    pub log_level: Option<String>,
    pub current_spinner: Option<String>,
    pub spinners: Option<HashMap<String, Spinner>>,
}

fn default_listen_addr() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    4040
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        let toml_content = r#"
            listen_addr = "127.0.0.1"
            port = 4040
            current_spinner = "minidot"

            [spinners]
            minidot = { interval = 83, frames = ["⠋", "⠙", "⠹"] }
            line = { interval = 100, frames = ["|", "/"] }
        "#;
        let config: Config = toml::from_str(toml_content).unwrap();
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.port, 4040);
        assert_eq!(config.current_spinner.as_deref(), Some("minidot"));

        let spinners = config.spinners.unwrap();
        let minidot = spinners.get("minidot").unwrap();
        assert_eq!(minidot.interval, 83);
        assert_eq!(
            minidot.frames,
            vec!["⠋".to_string(), "⠙".to_string(), "⠹".to_string()]
        );
    }
}
