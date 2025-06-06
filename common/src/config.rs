use crate::errors::ConfigError;
use alloy_json_abi::JsonAbi;
use anyhow::{Context, Result};
use chrono;
use config;
use log::debug;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::{fs, path::Path};
use tracing_appender::rolling;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const DEFAULT_LOG_DIRECTORY: &str = "logs";
const DEFAULT_TRACING_LEVEL: &str = "DEBUG";
const DEFAULT_DATE_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f";

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

#[derive(Debug, Deserialize)]
pub struct TracingConfig {
    pub log_directory: Option<String>,
    pub logfile_prefix: Option<String>,
    pub tracing_level: Option<String>,
    pub date_time_format: Option<String>,
    pub filtered_crates: Option<HashMap<String, String>>,
}

impl TracingConfig {
    /// Get log directory with default if None
    pub fn get_log_directory(&self) -> String {
        self.log_directory
            .clone()
            .unwrap_or_else(|| DEFAULT_LOG_DIRECTORY.to_string())
    }

    /// Get logfile prefix with default if None
    pub fn get_logfile_prefix(&self) -> String {
        self.logfile_prefix.clone().unwrap_or_else(|| String::new())
    }

    /// Get tracing level with default if None
    pub fn get_tracing_level(&self) -> String {
        self.tracing_level
            .clone()
            .unwrap_or_else(|| DEFAULT_TRACING_LEVEL.to_string())
    }

    /// Get date time format with default if None
    pub fn get_date_time_format(&self) -> String {
        self.date_time_format
            .clone()
            .unwrap_or_else(|| DEFAULT_DATE_TIME_FORMAT.to_string())
    }

    /// Get filtered crates with default if None
    pub fn get_filtered_crates(&self) -> HashMap<String, String> {
        self.filtered_crates
            .clone()
            .unwrap_or_else(|| HashMap::new())
    }
}

// Custom time formatter that uses configurable format
struct CustomTimeFormatter {
    format: String,
}

impl CustomTimeFormatter {
    fn new(format: String) -> Self {
        Self { format }
    }
}

impl FormatTime for CustomTimeFormatter {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = chrono::Local::now();
        write!(w, "{}", now.format(&self.format))
    }
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
            let abi_data = fs::read_to_string(&abi_path)
                .expect(&format!("Failed to read ABI file: {:?}", abi_full_path));
            Some(
                serde_json::from_str::<JsonAbi>(&abi_data)
                    .expect(&format!("Failed to parse ABI file: {:?}", abi_full_path)),
            )
        } else {
            debug!(
                "ABI file not found: {:?}. ABI will not be loaded.",
                abi_path
            );
            None
        }
    }

    pub fn init_tracer(
        config_path: String,
    ) -> Result<(tracing_appender::non_blocking::WorkerGuard, TracingConfig)> {
        // Read tracing configuration from file
        println!("Initializing tracing config from {:?}", config_path);
        let tracing_config = Self::load_tracing_config(&config_path)?;

        println!("Tracing config applied: {:?}", tracing_config);

        let debug_file = rolling::daily(
            &tracing_config.get_log_directory(),
            &tracing_config.get_logfile_prefix(),
        );

        let (non_blocking, guard) = tracing_appender::non_blocking(debug_file);

        let time_formatter = CustomTimeFormatter::new(tracing_config.get_date_time_format());

        let base_level = tracing_config.get_tracing_level().to_lowercase();
        let filtered_crates = tracing_config.get_filtered_crates();

        let mut filter: EnvFilter = EnvFilter::from_default_env()
            .add_directive(base_level.parse().expect("Invalid base tracing level"));

        for (crate_name, level) in filtered_crates {
            let directive = format!("{}={}", crate_name, level);
            filter =
                filter.add_directive(directive.parse().expect("Invalid crate filter directive"));
        }

        println!(
            "Tracing filter applied with base level: {} and {} crate-specific filters",
            tracing_config.get_tracing_level(),
            tracing_config.get_filtered_crates().len()
        );

        let result = tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_timer(time_formatter)
                    .with_line_number(true),
            )
            .try_init();

        match result {
            Ok(_) => println!("Tracing subscriber initialized successfully"),
            Err(e) => {
                // This is expected in tests where multiple tests try to set the global subscriber
                println!(
                    "Tracing subscriber already initialized (likely in test environment): {}",
                    e
                );
            }
        }

        Ok((guard, tracing_config))
    }

    fn load_tracing_config(config_path: &str) -> Result<TracingConfig, ConfigError> {
        let cfg = config::Config::builder()
            .add_source(config::File::with_name(config_path).required(true))
            .build()
            .map_err(ConfigError::ConfigFileError)?
            .try_deserialize::<TracingConfig>()
            .map_err(ConfigError::ConfigFileError)?;

        Ok(cfg)
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
            "Applied  to {:?}",
            format!("{}/{}.log", default_destination, crate_name)
        );

        let config = serde_yaml::from_str(&config_str).context("Failed to parse log4rs config")?;
        log4rs::init_raw_config(config).context("Failed to initialize log4rs")
    }
}
