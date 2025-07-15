use crate::errors::ConfigError;
use alloy_json_abi::JsonAbi;
use anyhow::{Context, Result};
use config;
use log::debug;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::{fs, path::Path};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
}

#[derive(Debug, Deserialize)]
pub struct IndexerConfig {
    pub initial_block_hash: String,
    pub sync: SyncConfig,
    pub storage: StorageConfig,
    pub cache: CacheConfig,
}

#[derive(Debug, Deserialize)]
pub struct NotifierConfig {
    pub broker_port: u16,
}

#[derive(Debug, Deserialize)]
pub struct SyncConfig {
    pub finality_depth: usize,
    pub batch_size: usize,
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
    // TODO(Jira-RethinkContractHandling) convert into a map
    pub name: String,
    pub address: String,
}

impl CommonConfig {
    pub fn load_config<T: DeserializeOwned>(
        path_opt: Option<&String>,
        crate_name: &str,
    ) -> Result<(T, String), ConfigError> {
        let config_path = match path_opt {
            Some(config_path) => config_path,
            None => &Self::get_default_config_path(),
        };

        let common_config = &format!("{config_path}/common.yaml");
        let crate_config = &format!("{config_path}/{crate_name}.yaml");

        println!(
            "Loading config from {:?} and {:?}",
            Path::new(common_config),
            Path::new(crate_config),
        );

        let cfg = config::Config::builder()
            .add_source(config::File::with_name(common_config).required(false)) // must exist if crate one does not
            .add_source(config::File::with_name(crate_config).required(false)) // must exist if common one does not
            .build()
            .map_err(ConfigError::ConfigFileError)?
            .try_deserialize::<T>()
            .map_err(ConfigError::ConfigFileError)?;

        Ok((cfg, config_path.to_string()))
    }

    pub fn get_default_config_path() -> String {
        let project_root = Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .and_then(|p| p.to_str())
            .expect("Failed to get default_config_path")
            .to_string();
        format!("{}/config/local", project_root)
    }

    pub fn load_abi_from_path(abi_path: &String) -> Option<JsonAbi> {
        if Path::new(&abi_path).exists() {
            let abi_full_path = Path::new(abi_path);
            let abi_data = fs::read_to_string(abi_path)
                .unwrap_or_else(|_| panic!("Failed to read ABI file: {:?}", abi_full_path));
            Some(
                serde_json::from_str::<JsonAbi>(&abi_data)
                    .unwrap_or_else(|_| panic!("Failed to parse ABI file: {:?}", abi_full_path)),
            )
        } else {
            debug!(
                "ABI file not found: {:?}. ABI will not be loaded.",
                abi_path
            );
            None
        }
    }

    pub fn init_logger(logger_file_opt: Option<&String>, crate_name: &str) -> Result<()> {
        // provided => use it as is
        if logger_file_opt.is_some() {
            let logger_file = logger_file_opt.unwrap();

            println!("Logging to destination defined by {logger_file}");

            log4rs::init_file(logger_file, Default::default())
                .context("Failed to load log4rs config")?;
            return Ok(());
        }

        // otherwise, use the default template and tweak it (mostly for local)
        let project_root = Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .and_then(|p| p.to_str())
            .expect("Failed to get default_destination");

        let base_yaml = format!("{project_root}/log4rs.yaml");
        let mut config_str = fs::read_to_string(&base_yaml)
            .context(format!("Failed to read base log4rs config: {base_yaml}"))?;

        let default_destination = &format!("{project_root}/logs");

        config_str = config_str.replace("{CRATE_NAME}", crate_name);
        config_str = config_str.replace("{DESTINATION}", default_destination);

        println!(
            "Logging to {:?}",
            format!("{}/{}.log", default_destination, crate_name)
        );

        let config = serde_yaml::from_str(&config_str).context("Failed to parse log4rs config")?;
        log4rs::init_raw_config(config).context("Failed to initialize log4rs")
    }
}
