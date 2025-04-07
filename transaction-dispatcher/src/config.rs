use common::config::{CommonConfig, ContractConfig, ProviderConfig};
use common::errors::ConfigError;
use common::types::{Address, ContractInfo};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub provider: ProviderConfig,
    pub key_store: KeyStoreConfig,
    pub server: ServerConfig,
    pub transaction: TransactionConfig,
    pub contracts: Vec<ContractConfig>,
    #[serde(skip)]
    path: String,
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

impl Config {
    pub fn load(base_path: &str) -> Result<Self, ConfigError> {
        let mut cfg = CommonConfig::load::<Self>(base_path, env!("CARGO_PKG_NAME"))?;
        cfg.path = base_path.to_string();
        Ok(cfg)
    }

    pub fn load_managed_contracts(&self) -> HashMap<String, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let address = Address::try_from(c.address.as_str())
                    .expect(&format!("Invalid address: {}", c.address));

                let abi_path = format!("{}/abi/{}.json", self.path, c.name);
                let abi = CommonConfig::load_abi_from_path(&abi_path);

                (
                    c.name.to_owned(),
                    ContractInfo {
                        name: c.name.to_owned(),
                        address,
                        abi,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    #[test]
    fn test_load_contracts_when_stage_config_set_should_load_contracts_successfully() {
        let config_path = format!("{}/../config/stage", CARGO_MANIFEST_DIR);
        let config: Config = Config::load(&config_path).expect("Failed to load config");
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
        assert!(!contract_info.abi.as_ref().unwrap().is_empty());

        // second contract
        let key = "TestContractCompiled";
        let contract_info = contracts.get(key).unwrap();

        assert_eq!(key, contract_info.name);
        assert_eq!(
            Address::try_from("0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc").unwrap(),
            contract_info.address
        );
        assert!(contract_info.abi.is_none());
    }
}
