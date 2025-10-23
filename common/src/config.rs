use crate::errors::ConfigError;
use anyhow::{Context, Result, bail};
use bitcoin::Network;
use config;
use config::{Environment, Source};
use log::trace;
use log4rs::config::RawConfig;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::{fs, path::Path};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
// todo(fede) replace the /new folder with the final folder
const BASE_CONFIG_PATH: &str = "config/base.yaml";
const ENV_CONFIG_PATH: &str = "config/environment";
// const ENV_CONFIG_PATH: &str = "new/environment/";

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
    pub bitcoin_network: String,
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
    pub fn load_config<T: DeserializeOwned>(env: Option<String>) -> Result<T, ConfigError> {
        let env = env.unwrap_or("".to_string());
        let (base_config_path, env_config_path) = Self::config_path_for(&env);

        trace!(
            "Loading config: base.yaml -> environment/{env}.yaml -> environment variables with prefix UB__"
        );

        let cfg = config::Config::builder()
            .add_source(config::File::with_name(&base_config_path).required(true))
            .add_source(config::File::with_name(&env_config_path).required(false))
            .add_source(
                Environment::with_prefix("UB")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(false)
                    .list_separator(";"),
            )
            .build()
            .map_err(ConfigError::ConfigFileError)?;

        trace!("Loaded config {:#?}", cfg.collect()?);

        let cfg_as_t = cfg
            .try_deserialize::<T>()
            .map_err(ConfigError::ConfigFileError)?;

        Ok(cfg_as_t)
    }

    fn config_path_for(env_name: &str) -> (String, String) {
        if env_name.is_empty() {
            trace!("Empty environment name");
        }

        let project_root = std::path::Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .expect("Failed to get project root");
        let env_config = format!("{ENV_CONFIG_PATH}/{env_name}.yaml");

        (
            project_root.join(BASE_CONFIG_PATH).display().to_string(),
            project_root.join(&env_config).display().to_string(),
        )
    }

    pub fn init_logger(logger_file_opt: Option<&String>, crate_name: &str) -> Result<()> {
        // provided => use it as is
        if let Some(logger_file) = logger_file_opt {
            trace!("Logging to destination defined by {logger_file}");

            let contents = fs::read_to_string(logger_file)?;
            let expanded = shellexpand::env(&contents)?.into_owned();
            let raw = serde_yaml::from_str::<RawConfig>(&expanded)?;

            log4rs::init_raw_config(raw).context("Failed to load log4rs config")?;

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

        trace!(
            "Logging to {:?}",
            format!("{}/{}.log", default_destination, crate_name)
        );

        let config = serde_yaml::from_str(&config_str).context("Failed to parse log4rs config")?;
        log4rs::init_raw_config(config).context("Failed to initialize log4rs")
    }

    pub fn parse_bitcoin_network(network_str: &str) -> Result<Network> {
        let res = match network_str {
            "bitcoin" | "mainnet" => Network::Bitcoin,
            "testnet" => Network::Testnet,
            "regtest" => Network::Regtest,
            _ => bail!("Invalid bitcoin network: {}", network_str),
        };

        Ok(res)
    }
}
