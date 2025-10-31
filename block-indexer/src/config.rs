use common::config::{CommonConfig, IndexerConfig, NotifierConfig, ProviderConfig};
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    #[serde(rename = "block-indexer")]
    pub block_indexer_config: BlockIndexerConfig,
}
#[derive(Debug, Deserialize)]
pub struct BlockIndexerConfig {
    pub block_notifier: NotifierConfig,
}

impl Config {
    pub fn load(env_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(env_name)
    }
}

pub struct Logger {}

impl Logger {
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{CARGO_PKG_NAME, Config};
    use common::config::CommonConfig;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_config_load_when_custom_config_set_should_load_config_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");

        assert_eq!(9001, config.block_indexer_config.block_notifier.broker_port);
    }

    #[test]
    fn test_init_logger_with_custom_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let logger_file = temp_dir.path().join("log4rs.yaml");
        let log_file = &format!("{}/{}", temp_dir.path().to_str().unwrap(), CARGO_PKG_NAME);

        let logger_config_template = r#"
refresh_rate: 30 seconds

appenders:
  rolling_file:
    kind: rolling_file
    path: "{TO_REPLACE}.log"
    encoder:
      pattern: "{d(%Y-%m-%d %H:%M:%S%.3f)} - {l:>5} - {m}{n}"
    policy:
      trigger:
        kind: size
        limit: 10mb
      roller:
        kind: fixed_window
        base: 1
        count: 5
        pattern: "{TO_REPLACE}.{}.log"

root:
  level: debug
  appenders:
    - rolling_file
"#;

        let logger_config_content = logger_config_template
            .to_string()
            .replace("{TO_REPLACE}", log_file);

        fs::write(&logger_file, logger_config_content).expect("Failed to write logger config");

        let result = CommonConfig::init_logger(
            Some(&logger_file.to_string_lossy().to_string()),
            "test_crate",
        );

        println!("result: {:?}", result);

        assert!(result.is_ok());
        assert!(Path::new(&format!("{log_file}.log")).exists());
    }
}
