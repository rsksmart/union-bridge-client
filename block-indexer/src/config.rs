use common::config::{CommonConfig, IndexerConfig, KeyStoreConfig, NotifierConfig, ProviderConfig};
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub key_store: KeyStoreConfig,
    #[serde(rename = "block_indexer")]
    pub block_indexer_config: BlockIndexerConfig,
}
#[derive(Debug, Deserialize)]
pub struct BlockIndexerConfig {
    pub notifier: NotifierConfig,
    pub broker_key_path: String,
}

impl Config {
    /// Load configuration from file
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the configuration file cannot be loaded or parsed
    pub fn load(config_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(config_name)
    }
}

pub struct Logger {}

impl Logger {
    /// Initialize logger
    ///
    /// # Errors
    ///
    /// Returns an error if the logger configuration file cannot be loaded or parsed
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use common::config::{CommonConfig, IndexerStartFrom};
    use tempfile::TempDir;

    use crate::config::{CARGO_PKG_NAME, Config};

    #[test]
    fn test_config_load_when_custom_config_set_should_load_config_successfully() {
        // base.toml no longer carries [[contracts]] or `environment`; per-env
        // overlays do, so load with an explicit profile.
        let config: Config = CommonConfig::load_config::<Config>(Some("local-anvil".to_string()))
            .expect("Failed to load config");

        assert_eq!(10001, config.block_indexer_config.notifier.port);
        // local-anvil overrides start_from to Best (base sets Hash).
        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert!(!config.indexer.storage.path.contains("{BASE_STORAGE_PATH}"));
        assert!(config.indexer.storage.path.ends_with("/.union_bridge/op_1/local_database"));
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
        assert!(
            config
                .block_indexer_config
                .broker_key_path
                .ends_with("/.union_bridge/op_1/union-client/broker/block-indexer.pem")
        );
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

        let logger_config_content =
            logger_config_template.to_string().replace("{TO_REPLACE}", log_file);

        fs::write(&logger_file, logger_config_content).expect("Failed to write logger config");

        let result = CommonConfig::init_logger(
            Some(&logger_file.to_string_lossy().to_string()),
            "test_crate",
        );

        println!("result: {result:?}");

        assert!(result.is_ok());
        assert!(Path::new(&format!("{log_file}.log")).exists());
    }
}
