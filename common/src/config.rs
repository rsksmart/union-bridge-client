use crate::{errors::ConfigError, types::ContractInfo};
use config;
use log::info;
use serde::Deserialize;
use std::{collections::HashMap, fs};
use yaml_rust::{yaml::Hash, Yaml, YamlLoader};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    #[serde(skip)]
    pub contracts: HashMap<String, ContractInfo>,
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

impl Config {
    pub fn load(config_path: &str) -> Result<Self, ConfigError> {
        let mut config = Self::parse_config(config_path)?;

        let contracts = Self::parse_contracts(config_path)?;
        config.contracts = contracts;

        Ok(config)
    }

    fn parse_config(config_path: &str) -> Result<Self, ConfigError> {
        let config_path = format!("{}/union-bridge-monitor.yaml", config_path);

        let config = config::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        config
            .try_deserialize::<Config>()
            .map_err(ConfigError::ConfigFileError)
    }

    fn parse_contracts(config_path: &str) -> Result<HashMap<String, ContractInfo>, ConfigError> {
        let contracts_path = format!("{}/contracts/contracts.yaml", config_path);
        let abi_path = format!("{}/contracts", config_path);

        let data = fs::read_to_string(&contracts_path).expect("Failed to read file data");
        let yaml_data = YamlLoader::load_from_str(&data).expect("Failed to parse YAML");

        let contracts = yaml_data
            .iter()
            .filter_map(Yaml::as_hash) // Extract top-level hash
            .flat_map(|hash| hash.iter())
            .filter_map(|(key, value)| {
                Self::parse_contract_info(key.as_str()?, value.as_hash()?, &abi_path)
            })
            .collect::<HashMap<_, _>>();

        info!(
            "Managed contracts: {:?}",
            contracts
                .iter()
                .map(|(address, contract)| format!("{} - {}", contract.name, address))
                .collect::<Vec<_>>()
        );

        Ok(contracts)
    }

    fn parse_contract_info(
        name: &str,
        fields: &Hash,
        abi_path: &str,
    ) -> Option<(String, ContractInfo)> {
        let address = fields
            .get(&Yaml::String("address".to_owned()))?
            .as_str()?
            .to_owned();
        let abi_file = fields
            .get(&Yaml::String("abi".to_owned()))
            .and_then(Yaml::as_str)
            .map(|abi_file| format!("{}/abi/{}", abi_path, abi_file));

        Some((
            address.clone(),
            ContractInfo {
                address,
                name: name.to_owned(),
                abi_file,
            },
        ))
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

        // Check config exists
        assert!(!config.indexer.initial_block_hash.is_empty());
        assert!(!config.indexer.storage.path.is_empty());
        assert!(config.indexer.cache.size > 0);
        assert!(!config.provider.rootstock.url.is_empty());

        // Check contracts config exists
        assert!(!config.contracts.is_empty());
        for (_, contract) in &config.contracts {
            assert!(!contract.name.is_empty());
            assert!(!contract.address.is_empty());
        }
    }
}
