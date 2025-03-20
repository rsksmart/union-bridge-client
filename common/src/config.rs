use crate::{
    errors::ConfigError,
    types::{Address, ContractInfo},
};
use alloy_json_abi::JsonAbi;
use config;
use serde::Deserialize;
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
    pub transaction_dispatcher: TransactionDispatcherConfig,
    #[serde(skip)]
    path: String,
}

#[derive(Debug, Deserialize)]
pub struct IndexerConfig {
    pub initial_block_hash: String,
    pub storage: StorageConfig,
    pub cache: CacheConfig,
}

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct CacheConfig {
    pub size: usize,
}

#[derive(Debug, Deserialize)]
pub struct ProviderConfig {
    pub rootstock: RootstockConfig,
}

#[derive(Debug, Deserialize)]
pub struct RootstockConfig {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ContractConfig {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Deserialize)]
pub struct TransactionDispatcherConfig {
    pub server_address: String,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let config_path = format!("{}/config.yaml", path);

        let raw_config = config::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        let mut parsed_config = raw_config
            .try_deserialize::<Config>()
            .map_err(ConfigError::ConfigFileError)?;

        parsed_config.path = path.to_owned();

        Ok(parsed_config)
    }

    pub fn load_managed_contracts(&self, by_name: bool) -> HashMap<String, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let address = Address::try_from(c.address.as_str())
                    .expect(&format!("Invalid address: {}", c.address));

                let abi_path = format!("{}/abi/{}.json", self.path, c.name);
                let abi = Self::load_abi_from_path(&abi_path);

                (
                    match by_name {
                        true => c.name.to_owned(),
                        false => c.address.to_owned(),
                    },
                    ContractInfo {
                        name: c.name.to_owned(),
                        address,
                        abi,
                    },
                )
            })
            .collect()
    }

    fn load_abi_from_path(abi_path: &String) -> Option<JsonAbi> {
        if Path::new(&abi_path).exists() {
            let abi_data = fs::read_to_string(&abi_path)
                .expect(&format!("Failed to read ABI file: {}", abi_path));
            Some(
                serde_json::from_str::<JsonAbi>(&abi_data)
                    .expect(&format!("Failed to parse ABI file: {}", abi_path)),
            )
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_config_load_when_stage_config_set_should_load_config_successfully() {
        let config_path = format!("{}/../config/stage", env!("CARGO_MANIFEST_DIR"));
        let config = Config::load(&config_path).expect("Failed to load config");

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
        let config_path = format!("{}/../config/stage", env!("CARGO_MANIFEST_DIR"));
        let config = Config::load(&config_path).expect("Failed to load config");
        let contracts = config.load_managed_contracts(true);

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

    #[test]
    fn test_load_contracts_when_dev_config_set_should_load_contracts_successfully_by_address() {
        let config_path = format!("{}/../config/stage", env!("CARGO_MANIFEST_DIR"));
        let config = Config::load(&config_path).expect("Failed to load config");
        let contracts = config.load_managed_contracts(false);

        assert_eq!(2, contracts.len());

        // first contract
        let key = "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761";
        let contract_info = contracts.get(key).unwrap();

        assert_eq!("TestContractDyn", contract_info.name);
        assert_eq!(key, contract_info.address);
        assert!(!contract_info.abi.as_ref().unwrap().is_empty());

        // second contract
        let key = "0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc";
        let contract_info = contracts.get(key).unwrap();

        assert_eq!("TestContractCompiled", contract_info.name);
        assert_eq!(key, contract_info.address);
        assert!(contract_info.abi.is_none());
    }
}
