use common::config::{CommonConfig, ContractConfig, IndexerConfig, ProviderConfig};
use common::errors::ConfigError;
use common::types::{Address, ContractInfo};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
    #[serde(skip)]
    pub path: String,
}

impl Config {
    pub fn load(base_path: &str) -> Result<Self, ConfigError> {
        let mut cfg = CommonConfig::load::<Self>(base_path, env!("CARGO_PKG_NAME"))?;
        cfg.path = base_path.to_string();
        Ok(cfg)
    }

    pub fn load_managed_contracts(&self) -> HashMap<Address, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let address = Address::try_from(c.address.as_str())
                    .expect(&format!("Invalid address: {}", c.address));

                let abi_path = format!("{}/abi/{}.json", self.path, c.name);
                let abi = CommonConfig::load_abi_from_path(&abi_path);

                (
                    address,
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
    use std::env;

    const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    #[test]
    fn test_config_load_when_stage_config_set_should_load_config_successfully() {
        let config_path = format!("{}/../config/stage", CARGO_MANIFEST_DIR);
        let config: Config = Config::load(&config_path).expect("Failed to load config");

        // indexer
        assert_eq!(
            "0xf6e292fd22f1dc5a1ef4022b7fe4a959f90ec0b9f5fc0869af64b99195511b22",
            config.indexer.initial_block_hash
        );
        assert_eq!(
            "/tmp/monitor-executions/default/storage",
            config.indexer.storage.path
        );
        assert_eq!(1000, config.indexer.cache.size);

        // provider
        assert_eq!(
            "ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket",
            config.provider.rootstock.url
        );

        // contracts
        assert_eq!(2, config.contracts.len());
        assert_eq!("TestContractDyn", config.contracts[0].name);
        assert_eq!(
            "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761",
            config.contracts[0].address
        );
        assert_eq!("TestContractCompiled", config.contracts[1].name);
        assert_eq!(
            "0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc",
            config.contracts[1].address
        );
    }

    #[test]
    fn test_load_contracts_when_stage_config_set_should_load_contracts_successfully() {
        let config_path = format!("{}/../config/stage", CARGO_MANIFEST_DIR);
        let config: Config = Config::load(&config_path).expect("Failed to load config");
        let contracts = config.load_managed_contracts();

        assert_eq!(2, contracts.len());

        // first contract
        let key = "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761";
        let address = Address::try_from(key).unwrap();
        let contract_info = contracts.get(&address).unwrap();

        assert_eq!("TestContractDyn", contract_info.name);
        assert_eq!(address, contract_info.address);
        assert!(!contract_info.abi.as_ref().unwrap().is_empty());

        // second contract
        let key = "0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc";
        let address = Address::try_from(key).unwrap();
        let contract_info = contracts.get(&address).unwrap();

        assert_eq!("TestContractCompiled", contract_info.name);
        assert_eq!(address, contract_info.address);
        assert!(contract_info.abi.is_none());
    }
}
