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
const BASE_CONFIG_PATH: &str = "config/base";
const ENV_CONFIG_PATH: &str = "config/environment";
const EXTENSION_TYPE: &str = "toml";

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

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderConfig {
    pub rootstock: RootstockConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RootstockConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ContractConfig {
    // TODO(Jira-RethinkContractHandling) convert into a map
    pub name: String,
    pub address: String,
}

impl CommonConfig {
    pub fn load_config<T: DeserializeOwned>(env: Option<String>) -> Result<T, ConfigError> {
        let env = env.unwrap_or("".to_string());
        let (base_config_path, env_config_path) = Self::config_path_for(&env)?;

        trace!(
            "Loading config: base.toml -> environment/{env}.toml -> environment variables with prefix UB__"
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

    fn config_path_for(env_name: &str) -> Result<(String, String), ConfigError> {
        if env_name.is_empty() {
            trace!("Empty environment name");
        }

        if env_name.contains("..") || env_name.contains('/') || env_name.contains('\\') {
            Err("{env_name}").map_err(|e| ConfigError::ConfigEnvError(e.to_string()))?;
        }

        let project_root = Self::project_root();
        let env_config = format!("{ENV_CONFIG_PATH}/{env_name}.{EXTENSION_TYPE}");

        Ok((
            format!("{project_root}/{BASE_CONFIG_PATH}.{EXTENSION_TYPE}"),
            format!("{project_root}/{env_config}"),
        ))
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
        let project_root = Self::project_root();

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

    fn project_root() -> String {
        let project_root = Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .and_then(|p| p.to_str())
            .expect("Failed to get default_destination");
        project_root.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::remove_var;
    use std::env::set_var;
    use std::sync::Mutex;

    // used to syncs tests that uses UB__ variables
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn cleanup_env_vars() {
        unsafe {
            let _ = remove_var("UB__INDEXER__STORAGE__PATH");
            let _ = remove_var("UB__INDEXER__CACHE__SIZE");
            let _ = remove_var("UB__PROVIDER__ROOTSTOCK__URL");
            let _ = remove_var("UB__BITCOIN_NETWORK");
        }
    }

    #[test]
    fn test_load_base_toml_config() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(None).expect("Failed to load base config");

        assert_eq!(
            "0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c",
            config.indexer.initial_block_hash
        );
        assert_eq!(
            "/your_base_path/.union_bridge/database/multi-client-1",
            config.indexer.storage.path
        );
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!(100, config.indexer.sync.finality_depth);
        assert_eq!(100, config.indexer.sync.batch_size);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(10, config.contracts.len());
        let contract_names: Vec<&String> = config.contracts.iter().map(|c| &c.name).collect();
        let expected_names = vec![
            "TestContractDyn",
            "TestContractCompiled",
            "PegManager",
            "SignatureManager",
            "CommitteeRegistry",
            "MemberRegistry",
            "FakePegManager",
            "FakeSignatureManager",
            "FakeCommitteeRegistry",
            "FakeMemberRegistry",
        ];
        assert_eq!(expected_names, contract_names);
        assert_eq!(
            "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761",
            config.contracts[0].address
        );
        assert_eq!(
            "0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc",
            config.contracts[1].address
        );
        assert_eq!(
            "0x1234567890123456789012345678901234567890",
            config.contracts[2].address
        );
        assert_eq!(
            "0x2345678901234567890123456789012345678901",
            config.contracts[3].address
        );
        assert_eq!(
            "0x3456789012345678901234567890123456789012",
            config.contracts[4].address
        );
        assert_eq!(
            "0x4567890123456789012345678901234567890123",
            config.contracts[5].address
        );
        assert_eq!(
            "0x5678901234567890123456789012345678901234",
            config.contracts[6].address
        );
        assert_eq!(
            "0x6789012345678901234567890123456789012345",
            config.contracts[7].address
        );
        assert_eq!(
            "0x7890123456789012345678901234567890123456",
            config.contracts[8].address
        );
        assert_eq!(
            "0x8901234567890123456789012345678901234567",
            config.contracts[9].address
        );
    }

    #[test]
    fn test_docker_local_environment_overrides() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("docker-local".to_string()))
                .expect("Failed to load config with docker-local environment");

        assert_eq!(
            "0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c",
            config.indexer.initial_block_hash
        );
        assert_eq!("/app/db/", config.indexer.storage.path); // override
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(8, config.contracts.len());
    }

    #[test]
    fn test_environment_variables_override_config_files() {
        let _guard = TEST_MUTEX.lock().unwrap();

        unsafe {
            set_var("UB__INDEXER__STORAGE__PATH", "/test/env/path");
            set_var("UB__INDEXER__CACHE__SIZE", "3000");
            set_var("UB__PROVIDER__ROOTSTOCK__URL", "ws://127.0.0.1:8888");
            set_var("UB__BITCOIN_NETWORK", "mainnet");
        }

        let config: CommonConfig = CommonConfig::load_config::<CommonConfig>(None)
            .expect("Failed to load config with environment variables");

        assert_eq!(
            "0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c",
            config.indexer.initial_block_hash
        );

        // override
        assert_eq!("/test/env/path", config.indexer.storage.path);
        assert_eq!(3000, config.indexer.cache.size);
        assert_eq!("ws://127.0.0.1:8888", config.provider.rootstock.url);
        assert_eq!("mainnet", config.bitcoin_network);

        cleanup_env_vars();
    }

    #[test]
    fn test_priority_order_base_env_file_env_vars() {
        let _guard = TEST_MUTEX.lock().unwrap();

        unsafe {
            set_var("UB__INDEXER__CACHE__SIZE", "3000");
            set_var("UB__BITCOIN_NETWORK", "mainnet");
        }

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("docker-local".to_string()))
                .expect("Failed to load config with all overrides");

        assert_eq!("/app/db/", config.indexer.storage.path); // environment override
        assert_eq!(3000, config.indexer.cache.size); // UB__ override
        assert_eq!("mainnet", config.bitcoin_network); // UB__ override

        cleanup_env_vars();
    }
}
