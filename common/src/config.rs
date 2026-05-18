use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use bitcoin::Network;
use config::{self, Environment, Source};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::{Event, Subscriber, info, trace};
pub use tracing_appender::non_blocking::WorkerGuard as LogGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, fmt};

use crate::errors::ConfigError;
use crate::rsk_provider::RskProvider;
use crate::types::{BlockHash, RskBlock};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const BASE_CONFIG_PATH: &str = "config/base";
const CONFIG_DIR_PATH: &str = "config";
const EXTENSION_TYPE: &str = "toml";
const LOG_DIR_ENV_VAR: &str = "UB_LOG_DIR";
const CLIENT_ID_ENV_VAR: &str = "CLIENT_ID";
const ENVIRONMENT_ENV_VAR: &str = "ENVIRONMENT";
const LOG_FORMAT_ENV_VAR: &str = "LOG_FORMAT";
const LOCAL_ENVIRONMENT: &str = "local";
/// Default per-crate log filters. Noisy third-party crates are pinned to `warn`
/// so they don't drown out service-level events; see `LOGGING_GUIDELINES.md`.
const DEFAULT_FILTER: &str = "debug,\
    tarpc=warn,\
    alloy_provider=warn,alloy_pubsub=warn,alloy_rpc_client=warn,alloy_json_rpc=warn,\
    hyper=warn,hyper_util=warn,h2=warn,\
    reqwest=warn,rustls=warn,tower_http=warn,tungstenite=warn";

/// Wraps an event formatter to prepend `[op-N]` to each stdout line. Used so that
/// when several services are launched in parallel under the local client launcher
/// (cli/run), their multiplexed output can be visually attributed to each operator.
struct PrefixedFormat<F> {
    prefix: Option<String>,
    inner: F,
}

impl<S, N, F> FormatEvent<S, N> for PrefixedFormat<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        if let Some(prefix) = &self.prefix {
            write!(writer, "[{prefix}] ")?;
        }
        self.inner.format_event(ctx, writer, event)
    }
}

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
    pub environment: String,
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub contracts: Vec<ContractConfig>,
    pub bitcoin_network: String,
}

#[derive(Debug, Deserialize)]
pub struct IndexerConfig {
    #[serde(default)]
    pub start_from: IndexerStartFrom,
    pub initial_block_hash: Option<String>,
    pub sync: SyncConfig,
    pub storage: StorageConfig,
    pub cache: CacheConfig,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IndexerStartFrom {
    #[default]
    Hash,
    Best,
}

#[derive(Debug, Deserialize)]
pub struct NotifierConfig {
    pub port: u16,
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
    pub name: String,
    pub address: String,
}
/// Key store configuration shared by all services.
/// Contains paths to keystores used for transaction signing.
#[derive(Debug, Deserialize, Clone)]
pub struct KeyStoreConfig {
    /// Path to user keystore (for user role transactions)
    pub user_path: String,
    /// Path to member keystore (for member role transactions)
    pub member_path: String,
}

impl IndexerConfig {
    /// Resolves the initial block number based on the `start_from` configuration.
    ///
    /// # Panics
    ///
    /// Panics when `start_from = "hash"` and `initial_block_hash` is missing or cannot be parsed
    /// as a valid block hash.
    ///
    /// # Errors
    ///
    /// Returns an error when:
    /// - `start_from = "hash"` and the provider fails to retrieve the block by hash, or the block
    ///   is not found on the provider.
    /// - `start_from = "best"` and the provider fails to retrieve the current best block.
    pub fn resolve_initial_block<P: RskProvider>(&self, provider: &P) -> Result<RskBlock> {
        let block = match self.start_from {
            IndexerStartFrom::Hash => {
                let hash_from_cfg = self.initial_block_hash.as_deref().context(
                    "Missing indexer.initial_block_hash when indexer.start_from is 'hash'",
                )?;

                let initial_block_hash = BlockHash::try_from(hash_from_cfg)
                    .with_context(|| format!("Invalid initial block hash: {hash_from_cfg}"))?;

                let block_by_hash = provider
                    .get_block_by_hash(initial_block_hash)
                    .context("Failed to get initial block by hash")?
                    .context("Initial block not found on provider")?;

                info!(
                    "Indexer start_from 'hash': using initial block {} ({})",
                    block_by_hash.hash(),
                    block_by_hash.number()
                );

                block_by_hash
            }
            IndexerStartFrom::Best => {
                let best_block = provider
                    .get_best_block()
                    .context("Failed to get best block for start_from='best'")?;

                info!(
                    "Indexer start_from 'best': using best block {} ({})",
                    best_block.hash(),
                    best_block.number()
                );

                best_block
            }
        };

        Ok(block)
    }
}

impl CommonConfig {
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed.
    pub fn load_config<T: DeserializeOwned>(config_name: Option<String>) -> Result<T, ConfigError> {
        let config_name = config_name.unwrap_or_default();
        let (base_config_path, config_profile_path) = Self::config_path_for(&config_name)?;

        trace!(
            "Loading config: base.toml -> {config_name}.toml -> environment variables with prefix UB__"
        );

        // load base config file with placeholder replacement
        let base_config = Self::read_and_process_config(&base_config_path)?;
        let mut builder = config::Config::builder()
            .add_source(config::File::from_str(&base_config, config::FileFormat::Toml));

        if !config_name.is_empty() && !Path::new(&config_profile_path).exists() {
            return Err(ConfigError::ConfigEnvError(format!(
                "Missing config profile '{config_name}' at {config_profile_path}"
            )));
        }

        // add environment-specific config if it exists
        if Path::new(&config_profile_path).exists() {
            let config_profile = Self::read_and_process_config(&config_profile_path)?;
            builder = builder
                .add_source(config::File::from_str(&config_profile, config::FileFormat::Toml));
        }

        // add environment variables and deserialize
        builder
            .add_source(
                Environment::with_prefix("UB")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(false)
                    .list_separator(";"),
            )
            .build()
            .and_then(|cfg| {
                trace!("Loaded config {:#?}", cfg.collect().ok());
                cfg.try_deserialize::<T>()
            })
            .map_err(ConfigError::ConfigFileError)
    }

    fn read_and_process_config(path: &str) -> Result<String, ConfigError> {
        fs::read_to_string(path).map(Self::replace_config_placeholders).map_err(|e| {
            ConfigError::ConfigEnvError(format!("Failed to read config from {path}: {e}"))
        })
    }

    fn replace_config_placeholders(mut config_str: String) -> String {
        // replace {BASE_STORAGE_PATH} with the environment variable value
        if config_str.contains("{BASE_STORAGE_PATH}") {
            let base_storage_path =
                std::env::var("BASE_STORAGE_PATH").unwrap_or_else(|_| ".".to_string());
            config_str = config_str.replace("{BASE_STORAGE_PATH}", &base_storage_path);
        }
        config_str
    }

    fn config_path_for(config_name: &str) -> Result<(String, String), ConfigError> {
        if config_name.is_empty() {
            trace!("Empty config name");
        }

        if config_name.contains("..") || config_name.contains('/') || config_name.contains('\\') {
            return Err(ConfigError::ConfigEnvError(format!(
                "Invalid configuration profile name: '{config_name}'. Profile names must not contain '..', '/', or '\\\\'."
            )));
        }

        let project_root = Self::project_root();
        let config_profile = format!("{CONFIG_DIR_PATH}/{config_name}.{EXTENSION_TYPE}");

        Ok((
            format!("{project_root}/{BASE_CONFIG_PATH}.{EXTENSION_TYPE}"),
            format!("{project_root}/{config_profile}"),
        ))
    }

    /// Initializes the tracing subscriber. See `LOGGING_GUIDELINES.md` for the rationale.
    ///
    /// **Format selection** (`LOG_FORMAT` env var, defaults derived from `ENVIRONMENT`):
    /// - `pretty` — human-readable colored output. Default when `ENVIRONMENT=local` (or unset).
    /// - `json`   — one JSON event per line, machine-parseable. Default in any other environment.
    ///
    /// **Outputs**:
    /// - Stdout: always emitted, with the chosen format.
    /// - File: written when `log_dir_opt` or `UB_LOG_DIR` is set, never with ANSI codes.
    ///
    /// **Operator identification**: when `CLIENT_ID` is set (injected per-operator by the
    /// cli/run launcher), the file name becomes `<crate_name>-<CLIENT_ID>.log`, and in pretty
    /// mode each stdout line is prefixed with `[op-<CLIENT_ID>] ` so launcher-interleaved
    /// output is attributable. Otherwise the file falls back to `<crate_name>-<timestamp>.log`.
    ///
    /// **Log level**: controlled by `RUST_LOG`. When unset, defaults to [`DEFAULT_FILTER`] —
    /// `debug` for service code, `warn` for noisy third-party crates.
    ///
    /// Also installs [`tracing_log::LogTracer`] so dependencies still using the `log` crate
    /// flow through the same subscriber.
    ///
    /// Returns a [`LogGuard`] that must be held alive for the duration of the process; dropping
    /// it flushes and closes the background file-writer thread.
    ///
    /// # Errors
    ///
    /// Returns an error only when the log directory cannot be created.
    pub fn init_logger(log_dir_opt: Option<&String>, crate_name: &str) -> Result<LogGuard> {
        // Bridge dependencies still using `log` macros into the tracing subscriber.
        // Err means it's already installed; harmless in tests where multiple inits run.
        let _ = tracing_log::LogTracer::init();

        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

        // Resolve log directory: CLI arg → UB_LOG_DIR env var → None (stdout only)
        let log_dir: Option<String> = log_dir_opt
            .cloned()
            .or_else(|| std::env::var(LOG_DIR_ENV_VAR).ok().filter(|s| !s.is_empty()));

        // Operator identifier injected by the local client launcher (cli/run).
        let client_id = std::env::var(CLIENT_ID_ENV_VAR).ok().filter(|s| !s.is_empty());

        // Environment + format selection. JSON in deployed environments, pretty in local dev.
        let environment = std::env::var(ENVIRONMENT_ENV_VAR)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| LOCAL_ENVIRONMENT.to_string());
        let json_format =
            std::env::var(LOG_FORMAT_ENV_VAR).ok().filter(|s| !s.is_empty()).map_or_else(
                || environment != LOCAL_ENVIRONMENT,
                |fmt| fmt.eq_ignore_ascii_case("json"),
            );

        // Build the file writer when a directory is configured; otherwise drain to sink.
        // With CLIENT_ID set, the parent directory is already per-execution (logs/YYMMDD/HHMMSS/)
        // and the launcher expects stable per-operator filenames, so no timestamp suffix.
        let (file_writer, guard) = if let Some(ref dir) = log_dir {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create log directory: {dir}"))?;
            let file_name = if let Some(id) = &client_id {
                format!("{crate_name}-{id}.log")
            } else {
                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                format!("{crate_name}-{timestamp}.log")
            };
            println!("Logging to file: {dir}/{file_name}");
            tracing_appender::non_blocking(tracing_appender::rolling::never(dir, file_name))
        } else {
            tracing_appender::non_blocking(std::io::sink())
        };
        let file_present = log_dir.is_some();

        // try_init returns Err when a global subscriber is already set (harmless in tests).
        if json_format {
            let stdout_layer = fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false);
            let file_layer = file_present.then(|| {
                fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_span_list(false)
                    .with_writer(file_writer)
                    .with_ansi(false)
            });
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .try_init()
                .ok();
        } else {
            // Pretty: stdout carries `[op-N]` when CLIENT_ID is set; file is plain.
            let stdout_layer = fmt::layer().event_format(PrefixedFormat {
                prefix: client_id.as_ref().map(|id| format!("op-{id}")),
                inner: fmt::format(),
            });
            let file_layer =
                file_present.then(|| fmt::layer().with_writer(file_writer).with_ansi(false));
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .try_init()
                .ok();
        }

        Ok(guard)
    }

    fn project_root() -> String {
        let project_root = Path::new(CARGO_MANIFEST_DIR)
            .parent()
            .and_then(|p| p.to_str())
            .expect("Failed to get default_destination");
        project_root.to_string()
    }

    /// # Errors
    ///
    /// Returns an error if the network string is invalid.
    pub fn parse_bitcoin_network(network_str: &str) -> Result<Network> {
        let res = match network_str {
            "bitcoin" | "mainnet" => Network::Bitcoin,
            "testnet" => Network::Testnet,
            "regtest" => Network::Regtest,
            _ => bail!("Invalid bitcoin network: {network_str}"),
        };

        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use std::env::{remove_var, set_var};
    use std::sync::Mutex;

    use super::*;

    // used to syncs tests that uses UB__ variables
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn cleanup_env_vars() {
        // SAFETY: Callers hold TEST_MUTEX, serializing process env mutation across tests.
        unsafe {
            remove_var("UB__INDEXER__STORAGE__PATH");
            remove_var("UB__INDEXER__CACHE__SIZE");
            remove_var("UB__PROVIDER__ROOTSTOCK__URL");
            remove_var("UB__BITCOIN_NETWORK");
        }
    }

    #[test]
    fn test_load_base_toml_config() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(None).expect("Failed to load base config");

        assert_eq!(
            Some("0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c"),
            config.indexer.initial_block_hash.as_deref()
        );
        assert_eq!(IndexerStartFrom::Hash, config.indexer.start_from);
        assert!(!config.indexer.storage.path.contains("{BASE_STORAGE_PATH}"));
        assert!(config.indexer.storage.path.ends_with("/.union_bridge/op_1/local_database"));
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
            "PeginManager",
            "PegoutManager",
            "SignatureManager",
            "CommitteeRegistry",
            "MemberRegistry",
            "StreamManager",
            "ChallengeManager",
            "NativeBridge",
        ];
        assert_eq!(expected_names, contract_names);
        assert_eq!("0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761", config.contracts[0].address);
        assert_eq!("0x9d4b2c05818A0086e641437fcb64ab6098c7BbEc", config.contracts[1].address);
        assert_eq!("0x9A9f2CCfdE556A7E9Ff0848998Aa4a0CFD8863AE", config.contracts[2].address);
        assert_eq!("0x3Aa5ebB10DC797CAC828524e59A333d0A371443c", config.contracts[3].address);
        assert_eq!("0x0B306BF915C4d645ff596e518fAf3F9669b97016", config.contracts[4].address);
        assert_eq!("0x0DCd1Bf9A1b36cE34237eEaFef220932846BCD82", config.contracts[5].address);
        assert_eq!("0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e", config.contracts[6].address);
        assert_eq!("0x0165878A594ca255338adfa4d48449f69242Eb8F", config.contracts[7].address);
        assert_eq!("0x59b670e9fA9D0A427751Af201D676719a970857b", config.contracts[8].address);
        assert_eq!("0x0000000000000000000000000000000001000006", config.contracts[9].address);
    }

    #[test]
    fn test_docker_environment_overrides() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("docker".to_string()))
                .expect("Failed to load config with docker environment");

        assert_eq!(
            Some("0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c"),
            config.indexer.initial_block_hash.as_deref()
        );
        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert_eq!("/app/db/", config.indexer.storage.path); // override
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!("ws://host.docker.internal:8545", config.provider.rootstock.url);
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(10, config.contracts.len());
    }

    #[test]
    fn test_environment_variables_override_config_files() {
        let _guard = TEST_MUTEX.lock().unwrap();

        // SAFETY: Access to process-global env vars is serialized via TEST_MUTEX (held above).
        unsafe {
            set_var("UB__INDEXER__STORAGE__PATH", "/test/env/path");
            set_var("UB__INDEXER__CACHE__SIZE", "3000");
            set_var("UB__PROVIDER__ROOTSTOCK__URL", "ws://127.0.0.1:8888");
            set_var("UB__BITCOIN_NETWORK", "mainnet");
        }

        let config: CommonConfig = CommonConfig::load_config::<CommonConfig>(None)
            .expect("Failed to load config with environment variables");

        assert_eq!(
            Some("0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c"),
            config.indexer.initial_block_hash.as_deref()
        );
        assert_eq!(IndexerStartFrom::Hash, config.indexer.start_from);

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

        // SAFETY: Access to process-global env vars is serialized via TEST_MUTEX (held above).
        unsafe {
            set_var("UB__INDEXER__CACHE__SIZE", "3000");
            set_var("UB__BITCOIN_NETWORK", "mainnet");
        }

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("docker".to_string()))
                .expect("Failed to load config with all overrides");

        assert_eq!("/app/db/", config.indexer.storage.path); // environment override
        assert_eq!(3000, config.indexer.cache.size); // UB__ override
        assert_eq!("mainnet", config.bitcoin_network); // UB__ override

        cleanup_env_vars();
    }

    #[test]
    fn test_explicit_missing_config_profile_errors() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let err = CommonConfig::load_config::<CommonConfig>(Some("alphanet".to_string()))
            .expect_err("missing explicit config profile should fail");

        match err {
            ConfigError::ConfigEnvError(message) => {
                assert!(message.contains("Missing config profile 'alphanet'"));
            }
            other @ ConfigError::ConfigFileError(_) => {
                panic!("unexpected error variant: {other:?}");
            }
        }
    }
}
