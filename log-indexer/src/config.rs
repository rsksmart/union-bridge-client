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
    /// Initialize logger.
    ///
    /// `log_dir_opt` is an optional directory for log files. When `None`, the
    /// `UB_LOG_DIR` env var is consulted; if neither is set, logs are written
    /// under `./logs/` (relative to the current working directory).
    ///
    /// Returns a [`common::config::LogGuard`] that must be kept alive for the
    /// duration of the process to flush the background file-writer thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the log directory cannot be created.
    pub fn init(log_dir_opt: Option<&String>) -> anyhow::Result<common::config::LogGuard> {
        CommonConfig::init_logger(log_dir_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use common::config::IndexerStartFrom;

    use super::*;

    #[test]
    fn test_config_load_when_stage_config_set_should_load_config_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");

        assert_eq!(
            Some("0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c"),
            config.indexer.initial_block_hash.as_deref()
        );
        assert_eq!(IndexerStartFrom::Hash, config.indexer.start_from);
        assert!(!config.indexer.storage.path.contains("{BASE_STORAGE_PATH}"));
        assert!(config.indexer.storage.path.ends_with("/.union_bridge/op_1/local_database"));
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
        assert_eq!(10, config.contracts.len());
        assert!(
            config
                .log_indexer_config
                .broker_key_path
                .ends_with("/.union_bridge/op_1/union-client/broker/log-indexer.pem")
        );
    }

    #[test]
    fn test_load_contracts_when_stage_config_set_should_load_contracts_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");
        let contracts = config.load_managed_contracts();

        assert_eq!(10, contracts.len());
    }

    #[test]
    fn test_init_logger() {
        // Smoke test: a different test in this binary may have already installed
        // a global subscriber, in which case init_logger legitimately Errs. We
        // only assert the call doesn't panic.
        let _ = CommonConfig::init_logger(None, CARGO_PKG_NAME);
    }
}
