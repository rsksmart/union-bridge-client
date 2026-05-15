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
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<common::config::LogGuard> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use common::config::{CommonConfig, IndexerStartFrom};

    use crate::config::{CARGO_PKG_NAME, Config};

    #[test]
    fn test_config_load_when_custom_config_set_should_load_config_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");

        assert_eq!(10001, config.block_indexer_config.notifier.port);
        assert_eq!(IndexerStartFrom::Hash, config.indexer.start_from);
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
    fn test_init_logger() {
        let _ = CARGO_PKG_NAME; // ensure the constant is still referenced
        let result = CommonConfig::init_logger(None, "test_crate");
        assert!(result.is_ok());
    }
}
