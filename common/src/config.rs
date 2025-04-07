use crate::errors::ConfigError;
use alloy_json_abi::JsonAbi;
use config;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
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

impl CommonConfig {
    pub fn load<T: DeserializeOwned>(base_path: &str, crate_name: &str) -> Result<T, ConfigError> {
        let common_config = format!("{base_path}/common.yaml");
        let config = format!("{base_path}/{crate_name}.yaml");
        config::Config::builder()
            .add_source(config::File::with_name(&common_config).required(false))
            .add_source(config::File::with_name(&config).required(false))
            .build()
            .map_err(ConfigError::ConfigFileError)?
            .try_deserialize::<T>()
            .map_err(ConfigError::ConfigFileError)
    }

    pub fn load_abi_from_path(abi_path: &String) -> Option<JsonAbi> {
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
