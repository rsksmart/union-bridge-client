use crate::{errors::ConfigError, types::ContractInfo};
use config;
use log::{info, warn};
use serde::Deserialize;
use std::{collections::HashMap, env, fs};
use yaml_rust::{yaml::Hash, Yaml, YamlLoader};

const DEFAULT_ENV: &str = "dev";
const CONFIG_PATH: &str = "config";

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
    pub store: StorageConfig,
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
    pub fn load() -> Result<Self, ConfigError> {
        let env = Self::get_env();

        let mut config = Self::parse_config(&env)?;

        let contracts = Self::parse_contracts(&env)?;
        config.contracts = contracts;

        Ok(config)
    }

    fn get_env() -> String {
        env::var("UNION_BRIDGE_MONITOR_ENV").unwrap_or_else(|_| {
            let default_env = DEFAULT_ENV.to_string();
            warn!(
                "UNION_BRIDGE_MONITOR_ENV not set. Using default environment: {}",
                default_env
            );
            default_env
        })
    }

    fn parse_config(env: &str) -> Result<Self, ConfigError> {
        let config_path = format!("{}/{}/union-bridge-monitor.yaml", CONFIG_PATH, env);

        let config = config::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        config
            .try_deserialize::<Config>()
            .map_err(ConfigError::ConfigFileError)
    }

    fn parse_contracts(env: &str) -> Result<HashMap<String, ContractInfo>, ConfigError> {
        let contracts_path = format!("{}/{}/contracts/contracts.yaml", CONFIG_PATH, env);
        let abi_path = format!("{}/{}/contracts", CONFIG_PATH, env);

        let data = fs::read_to_string(&contracts_path).expect("Failed to read file dat");
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
