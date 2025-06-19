use common::config::{
    CommonConfig, ContractConfig, IndexerConfig, NotifierConfig, ProviderConfig, TracingConfig,
};
use common::errors::ConfigError;
use common::types::{Address, ContractInfo};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing_appender::non_blocking::WorkerGuard;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

// Global static to hold the WorkerGuard for the entire program lifetime
static TRACER_GUARD: OnceLock<Option<WorkerGuard>> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub notifier: NotifierConfig,
    pub contracts: Vec<ContractConfig>,
    #[serde(skip)]
    pub path: String,
}

impl Config {
    pub fn load(base_path: Option<&String>) -> Result<Self, ConfigError> {
        let (mut cfg, path) = CommonConfig::load_config::<Self>(base_path, CARGO_PKG_NAME)?;
        cfg.path = path;
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

pub struct Tracer {
    config: TracingConfig,
}

// Manual Debug implementation since WorkerGuard doesn't implement Debug
impl std::fmt::Debug for Tracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracer")
            .field("_guard", &"<Global WorkerGuard>")
            .field("config", &self.config)
            .finish()
    }
}

impl Tracer {
    pub fn init(logger_path: Option<&String>) -> anyhow::Result<Self> {
        let path = logger_path.cloned().unwrap_or_else(|| {
            format!(
                "{}/log-indexer-tracing.yaml",
                CommonConfig::get_default_config_path()
            )
        });

        let (guard, config) = CommonConfig::init_tracer(path, CARGO_PKG_NAME)?;

        // Store the guard in the global static - this will keep it alive for the entire program
        let _ = TRACER_GUARD.set(guard);

        Ok(Tracer { config })
    }

    #[cfg(test)]
    pub fn get_config(&self) -> &TracingConfig {
        &self.config
    }

    #[cfg(test)]
    pub fn log_directory(&self) -> String {
        self.config.get_log_directory()
    }

    #[cfg(test)]
    pub fn logfile_prefix(&self) -> String {
        self.config.get_logfile_prefix()
    }

    #[cfg(test)]
    pub fn tracing_level(&self) -> String {
        self.config.get_tracing_level()
    }

    #[cfg(test)]
    pub fn date_time_format(&self) -> String {
        self.config.get_date_time_format()
    }
}

pub struct Logger {}

impl Logger {
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::{env, fs};
    use tempfile::TempDir;

    const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    #[test]
    fn test_config_load_when_stage_config_set_should_load_config_successfully() {
        let config_path = &format!("{}/tests/config", CARGO_MANIFEST_DIR);
        let config: Config = Config::load(Some(config_path)).expect("Failed to load config");

        // indexer
        assert_eq!(
            "0xf6e292fd22f1dc5a1ef4022b7fe4a959f90ec0b9f5fc0869af64b99195511b22",
            config.indexer.initial_block_hash
        );
        assert_eq!("/fake/storage", config.indexer.storage.path);
        assert_eq!(1000, config.indexer.cache.size);

        // provider
        assert_eq!(
            "ws://fake-server:4445/websocket",
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
        let config_path = &format!("{}/tests/config", CARGO_MANIFEST_DIR);
        let config: Config = Config::load(Some(&config_path)).expect("Failed to load config");
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

    #[test]
    fn test_init_logger_with_custom_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let logger_file = temp_dir.path().join("log4rs.yaml");
        let log_file = &format!("{}/{}", temp_dir.path().to_str().unwrap(), CARGO_PKG_NAME);

        let logger_config_template = r#"
refresh_rate: 30 seconds

appenders:
  rolling_file:
    kind: rolling_file
    path: "{TO_REPLACE}.log"
    encoder:
      pattern: "{d(%Y-%m-%d %H:%M:%S%.3f)} - {l} - {m}{n}"
    policy:
      trigger:
        kind: size
        limit: 10mb
      roller:
        kind: fixed_window
        base: 1
        count: 5
        pattern: "{TO_REPLACE}.{}.log"

root:
  level: debug
  appenders:
    - rolling_file
"#;

        let logger_config_content = logger_config_template
            .to_string()
            .replace("{TO_REPLACE}", log_file);

        fs::write(&logger_file, logger_config_content).expect("Failed to write logger config");

        let result = CommonConfig::init_logger(
            Some(&logger_file.to_string_lossy().to_string()),
            "test_crate",
        );

        println!("result: {:?}", result);

        assert!(result.is_ok());
        assert!(Path::new(&format!("{log_file}.log")).exists());
    }

    #[test]
    fn test_tracer_init_with_provided_path() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("custom_tracing.yaml");
        let log_dir = temp_dir.path().join("custom_logs");
        fs::create_dir_all(&log_dir).expect("Failed to create log directory");

        let config_content = format!(
            r#"
log_directory: "{}"
logfile_prefix: "custom_tracer"
tracing_level: "WARN"
"#,
            log_dir.to_str().unwrap()
        );

        fs::write(&config_file, config_content).expect("Failed to write config file");

        let config_path = config_file.to_str().unwrap().to_string();
        let result = Tracer::init(Some(&config_path));

        assert!(result.is_ok());
        let tracer = result.unwrap();

        // Verify the applied configuration
        assert_eq!(tracer.log_directory(), log_dir.to_str().unwrap());
        assert_eq!(tracer.logfile_prefix(), "custom_tracer");
        assert_eq!(tracer.tracing_level(), "WARN");
        assert_eq!(tracer.date_time_format(), "%Y-%m-%d %H:%M:%S%.3f"); // default
    }

    #[test]
    fn test_tracer_init_with_default_values() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("defaults_tracing.yaml");

        // Create default logs directory
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).expect("Failed to create logs directory");

        // Config with minimal content - should use defaults
        let config_content = r#"
# Minimal config, most fields will use defaults
log_directory: "./logs"
"#;

        fs::write(&config_file, config_content).expect("Failed to write config file");

        // Change to temp directory so relative path works
        let original_dir = std::env::current_dir().expect("Failed to get current dir");
        std::env::set_current_dir(&temp_dir).expect("Failed to change directory");

        let config_path = config_file.to_str().unwrap().to_string();
        let result = Tracer::init(Some(&config_path));

        // Restore original directory
        std::env::set_current_dir(original_dir).expect("Failed to restore directory");

        assert!(result.is_ok());
        let tracer = result.unwrap();

        // Verify the applied configuration with defaults
        assert_eq!(tracer.log_directory(), "./logs");
        assert_eq!(tracer.logfile_prefix(), ""); // default
        assert_eq!(tracer.tracing_level(), "DEBUG"); // default
        assert_eq!(tracer.date_time_format(), "%Y-%m-%d %H:%M:%S%.3f"); // default
    }
}
