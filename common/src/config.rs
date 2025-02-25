use crate::{errors::ConfigError, types::ContractInfo};
use config;
use serde::Deserialize;
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
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
    pub size: u32,
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

    pub fn load_contracts(&self) -> HashMap<String, ContractInfo> {
        self.contracts
            .iter()
            .map(|c| {
                let abi_path = format!("{}/contracts/{}.json", self.path, c.address);
                let abi_data = if Path::new(&abi_path).exists() {
                    Some(
                        fs::read_to_string(&abi_path)
                            .expect(&format!("Failed to read ABI file: {}", abi_path)),
                    )
                } else {
                    None
                };

                (
                    c.address.to_owned(),
                    ContractInfo {
                        name: c.name.to_owned(),
                        address: c.address.to_owned(),
                        abi: abi_data,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use std::env;

    #[test]
    fn test_config_load_when_dev_config_set_should_load_config_successfully() {
        let config_path = format!("{}/../config/dev", env!("CARGO_MANIFEST_DIR"));
        let config = Config::load(&config_path).expect("Failed to load config");

        // indexer
        assert_eq!(
            "<YOUR_INITIAL_BLOCK_HASH_HERE>",
            config.indexer.initial_block_hash
        );
        assert_eq!("<YOUR_DB_PATH_HERE>", config.indexer.storage.path);
        assert_eq!(1000, config.indexer.cache.size);

        // provider
        assert_eq!(
            "wss://public-node.testnet.rsk.co/websocket",
            config.provider.rootstock.url
        );

        // contracts
        assert_eq!(2, config.contracts.len());
        assert_eq!(
            "RootstockTestnetMultiFeedAdapterWithoutRoundsV1",
            config.contracts[0].name
        );
        assert_eq!(
            "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761",
            config.contracts[0].address
        );
        assert_eq!("MoCMedianizer", config.contracts[1].name);
        assert_eq!(
            "0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc",
            config.contracts[1].address
        );
    }
}
