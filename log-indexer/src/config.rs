use std::collections::HashMap;

use common::config::{
    CommonConfig, ContractConfig, IndexerConfig, KeyStoreConfig, NotifierConfig, ProviderConfig,
};
use common::errors::ConfigError;
use common::types::{Address, ContractInfo};
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
    pub key_store: KeyStoreConfig,
    #[serde(rename = "log_indexer")]
    pub log_indexer_config: LogIndexerConfig,
}

#[derive(Debug, Deserialize)]
pub struct LogIndexerConfig {
    pub notifier: NotifierConfig,
}

impl Config {
    /// Load configuration from file
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the configuration file cannot be loaded or parsed
    pub fn load(env_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(env_name)
    }

    /// Load managed contracts from configuration
    ///
    /// # Panics
    ///
    /// Panics if any contract address in the configuration is invalid
    #[must_use]
    pub fn load_managed_contracts(&self) -> HashMap<Address, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let address = Address::try_from(c.address.as_str())
                    .unwrap_or_else(|_| panic!("Invalid address: {}", c.address));
                (address, ContractInfo { name: c.name.clone(), address })
            })
            .collect()
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

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_config_load_when_stage_config_set_should_load_config_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");

        assert_eq!(
            "0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c",
            config.indexer.initial_block_hash
        );
        assert!(!config.indexer.storage.path.contains("{BASE_STORAGE_PATH}"));
        assert!(config.indexer.storage.path.ends_with("/.union_bridge/database/multi-client-1"));
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
        assert_eq!(11, config.contracts.len());
    }

    #[test]
    fn test_load_contracts_when_stage_config_set_should_load_contracts_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");
        let contracts = config.load_managed_contracts();

        assert_eq!(11, contracts.len());
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
