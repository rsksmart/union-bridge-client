use common::config::{CommonConfig, ContractConfig, ProviderConfig};
use common::errors::ConfigError;
use common::types::{Address, ContractInfo};
use serde::Deserialize;
use std::collections::HashMap;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct ConfigAsBin {
    pub server: ServerConfig,
    #[serde(flatten)]
    pub lib_config: ConfigAsLib,
}

#[derive(Debug, Deserialize)]
pub struct ConfigAsLib {
    pub provider: ProviderConfig,
    pub key_store: KeyStoreConfig,
    pub transaction: TransactionConfig,
    pub contracts: Vec<ContractConfig>,
}

#[derive(Debug, Deserialize)]
pub struct KeyStoreConfig {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct TransactionConfig {
    pub gas_bumps_t1: u8,
}

impl ConfigAsBin {
    pub fn load(base_path: Option<&String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<ConfigAsBin>(base_path, CARGO_PKG_NAME)
    }

    pub fn provider(&self) -> &ProviderConfig {
        &self.lib_config.provider
    }

    pub fn load_managed_contracts(&self) -> HashMap<String, ContractInfo> {
        self.lib_config.load_managed_contracts()
    }

    pub fn key_store(&self) -> &KeyStoreConfig {
        &self.lib_config.key_store
    }

    pub fn transaction(&self) -> &TransactionConfig {
        &self.lib_config.transaction
    }

    pub fn contracts(&self) -> &Vec<ContractConfig> {
        &self.lib_config.contracts
    }
}

impl ConfigAsLib {
    pub fn load(base_path: Option<&String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<ConfigAsLib>(base_path, CARGO_PKG_NAME)
    }

    pub fn load_managed_contracts(&self) -> HashMap<String, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let address = Address::try_from(c.address.as_str())
                    .expect(&format!("Invalid address: {}", c.address));
                (
                    c.name.to_owned(),
                    ContractInfo {
                        name: c.name.to_owned(),
                        address,
                        abi: None,
                    },
                )
            })
            .collect()
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
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    #[test]
    fn test_config_load_when_custom_config_set_should_load_config_successfully() {
        let config_path = &format!("{}/tests/config", CARGO_MANIFEST_DIR);
        let config: ConfigAsBin =
            ConfigAsBin::load(Some(config_path)).expect("Failed to load config");

        // provider
        assert_eq!(
            "ws://fake-server:4445/websocket",
            config.provider().rootstock.url
        );

        // server
        assert_eq!("0.0.0.0:3000", config.server.url);

        // key store
        assert_eq!(
            "/fake/path/.union_bridge/keystore/ab98c3c8-a772-4c44-b3a7-99864663b8cb",
            config.key_store().path
        );

        // transaction
        assert_eq!(3, config.transaction().gas_bumps_t1);
    }

    #[test]
    fn test_load_contracts_when_stage_config_set_should_load_contracts_successfully() {
        let config_path = &format!("{}/tests/config", CARGO_MANIFEST_DIR);
        let config: ConfigAsBin =
            ConfigAsBin::load(Some(config_path)).expect("Failed to load config");
        let contracts = config.load_managed_contracts();

        assert_eq!(2, contracts.len());

        // first contract
        let key = "TestContractDyn";
        let contract_info = contracts.get(key).unwrap();

        assert_eq!(key, contract_info.name);
        assert_eq!(
            Address::try_from("0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761").unwrap(),
            contract_info.address
        );

        // second contract
        let key = "TestContractCompiled";
        let contract_info = contracts.get(key).unwrap();

        assert_eq!(key, contract_info.name);
        assert_eq!(
            Address::try_from("0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc").unwrap(),
            contract_info.address
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
