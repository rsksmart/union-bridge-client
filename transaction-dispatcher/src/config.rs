use std::collections::HashMap;

use common::config::{CommonConfig, ContractConfig, KeyStoreConfig, ProviderConfig};
use common::errors::ConfigError;
use common::types::{Address, ContractInfo};
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
    pub key_store: KeyStoreConfig,
    #[serde(rename = "transaction_dispatcher")]
    pub tx_dispatcher_config: TxDispatcherConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TxDispatcherConfig {
    pub transaction: TransactionConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TransactionConfig {
    pub gas_bumps_t1: u8,
}

impl Config {
    /// Load configuration from file
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the configuration file cannot be loaded or parsed
    pub fn load(config_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Config>(config_name)
    }

    /// Load managed contracts from configuration
    ///
    /// # Panics
    ///
    /// Panics if any contract address in the configuration is invalid
    #[must_use]
    pub fn load_managed_contracts(&self) -> HashMap<String, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let address = Address::try_from(c.address.as_str())
                    .unwrap_or_else(|_| panic!("Invalid address: {}", c.address));
                (c.name.clone(), ContractInfo { name: c.name.clone(), address })
            })
            .collect()
    }

    #[must_use]
    pub fn transaction(&self) -> &TransactionConfig {
        &self.tx_dispatcher_config.transaction
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
    use super::*;

    #[test]
    fn test_config_load_when_custom_config_set_should_load_config_successfully() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load config");

        // key store (now shared at top level)
        assert!(!config.key_store.user_path.contains("{BASE_STORAGE_PATH}"));
        assert!(
            config.key_store.user_path.ends_with("/.union_bridge/op_1/union-client/keystore/user")
        );
        assert!(!config.key_store.member_path.contains("{BASE_STORAGE_PATH}"));
        assert!(
            config
                .key_store
                .member_path
                .ends_with("/.union_bridge/op_1/union-client/keystore/member")
        );
        assert_eq!(3, config.transaction().gas_bumps_t1);
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
        let result = CommonConfig::init_logger(None, "test_crate");
        assert!(result.is_ok());
    }
}
