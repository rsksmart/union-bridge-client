use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use bitcoin::Network;
use config::{self, Environment, Source};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::{info, trace};
pub use tracing_appender::non_blocking::WorkerGuard as LogGuard;
use tracing_subscriber::layer::SubscriberExt as _;
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
/// Default log directory used when neither `--log-dir` nor `UB_LOG_DIR` is set.
/// Resolved relative to the process's current working directory.
const DEFAULT_LOG_DIR: &str = "logs";
/// Default per-crate log filters. Noisy third-party crates are pinned to `warn`
/// so they don't drown out service-level events; see `docs/LOGGING.md`.
const DEFAULT_FILTER: &str = "debug,\
    tarpc=warn,\
    alloy_provider=warn,alloy_pubsub=warn,alloy_rpc_client=warn,alloy_json_rpc=warn,\
    hyper=warn,hyper_util=warn,h2=warn,\
    reqwest=warn,rustls=warn,tower_http=warn,tungstenite=warn";

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
    /// Runtime tier classification used for cross-cutting policy
    /// (force flags, fake native bridge, signaling backend).
    /// Required: every overlay must set this. base.toml is incomplete on its own.
    pub environment: String,
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    /// `[[contracts]]` blocks live in the per-env overlay TOMLs, not in
    /// `base.toml`. Default to empty so the base layer can be deserialized
    /// on its own (e.g. in tests that exercise env-var overrides) without
    /// requiring a contracts list to exist there.
    #[serde(default)]
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

    /// Initializes the tracing subscriber. See `docs/LOGGING.md` for the rationale.
    ///
    /// **Format selection** (`LOG_FORMAT` env var, defaults derived from `ENVIRONMENT`):
    /// - `pretty` — human-readable colored output. Default when `ENVIRONMENT=local` (or unset).
    /// - `json`   — one JSON event per line, machine-parseable. Default in any other environment.
    ///
    /// **Outputs**:
    /// - Stdout: always emitted, with the chosen format.
    /// - File: always written, never with ANSI codes. The directory is resolved as
    ///   `log_dir_opt` (CLI arg) → `UB_LOG_DIR` env var → `DEFAULT_LOG_DIR`
    ///   (relative to the current working directory).
    ///
    /// **Operator identification**: when `CLIENT_ID` is set (injected per-operator by the
    /// cli/run launcher), the file name becomes `<crate_name>-<CLIENT_ID>.log` so the
    /// per-operator log file is stable. Otherwise the file falls back to
    /// `<crate_name>-<timestamp>.log`.
    ///
    /// **Log level**: controlled by `RUST_LOG`. When unset, defaults to `DEFAULT_FILTER` —
    /// `debug` for service code, `warn` for noisy third-party crates.
    ///
    /// Also installs [`tracing_log::LogTracer`] (via `tracing-subscriber`'s default
    /// `tracing-log` feature) so dependencies still using the `log` crate flow through
    /// the same subscriber.
    ///
    /// Returns a [`LogGuard`] that must be held alive for the duration of the process; dropping
    /// it flushes and closes the background file-writer thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the log directory cannot be created, or if a global tracing
    /// subscriber has already been installed (e.g. in tests that call this more than once).
    pub fn init_logger(log_dir_opt: Option<&String>, crate_name: &str) -> Result<LogGuard> {
        // `tracing-subscriber` enables the `tracing-log` default feature, so `try_init()`
        // installs `LogTracer` internally. An explicit call here would double-register it
        // and cause try_init() to return "logger already initialized".

        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

        let log_dir = Self::resolve_log_dir(
            log_dir_opt.map(String::as_str),
            std::env::var(LOG_DIR_ENV_VAR).ok().as_deref(),
        );
        let client_id = std::env::var(CLIENT_ID_ENV_VAR).ok().filter(|s| !s.is_empty());
        let json_format = Self::select_json_format(
            std::env::var(LOG_FORMAT_ENV_VAR).ok().as_deref(),
            std::env::var(ENVIRONMENT_ENV_VAR).ok().as_deref(),
        );

        // With CLIENT_ID set, the parent directory is already per-execution
        // (logs/YYMMDD/HHMMSS/) and the launcher expects stable per-operator filenames,
        // so no timestamp suffix.
        std::fs::create_dir_all(&log_dir)
            .with_context(|| format!("Failed to create log directory: {log_dir}"))?;
        let file_name =
            Self::build_log_file_name(crate_name, client_id.as_deref(), &chrono::Local::now());
        println!("Logging to file: {log_dir}/{file_name}");
        let (file_writer, guard) =
            tracing_appender::non_blocking(tracing_appender::rolling::never(&log_dir, file_name));

        // try_init returns Err when a global subscriber is already set. In production
        // that means the configured filter/layers will not be applied, so we surface
        // the error instead of silently dropping it.
        let init_result = if json_format {
            let stdout_layer = fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false);
            let file_layer = fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false)
                .with_writer(file_writer)
                .with_ansi(false);
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .try_init()
        } else {
            let file_layer = fmt::layer().with_writer(file_writer).with_ansi(false);
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .with(file_layer)
                .try_init()
        };

        init_result.context("Failed to install tracing subscriber")?;

        Ok(guard)
    }

    /// Resolves the log directory used by [`init_logger`]. CLI arg wins, then
    /// `UB_LOG_DIR`, then `DEFAULT_LOG_DIR`. Empty strings are treated as unset.
    fn resolve_log_dir(arg: Option<&str>, env: Option<&str>) -> String {
        arg.filter(|s| !s.is_empty())
            .or(env.filter(|s| !s.is_empty()))
            .map_or_else(|| DEFAULT_LOG_DIR.to_string(), str::to_string)
    }

    /// Decides whether to use JSON output. `LOG_FORMAT` is the explicit override
    /// (case-insensitive `json` ⇒ true, anything else ⇒ false). When unset,
    /// falls back to `ENVIRONMENT`: `local` (or unset) ⇒ pretty, anything else ⇒ JSON.
    /// Empty strings are treated as unset.
    fn select_json_format(log_format: Option<&str>, environment: Option<&str>) -> bool {
        match log_format.filter(|s| !s.is_empty()) {
            Some(fmt) => fmt.eq_ignore_ascii_case("json"),
            None => {
                environment.filter(|s| !s.is_empty()).is_some_and(|env| env != LOCAL_ENVIRONMENT)
            }
        }
    }

    /// Builds the log file name. With `CLIENT_ID` (operator launcher), the
    /// parent directory is already per-execution so we use a stable
    /// `<crate>-<id>.log` name; otherwise we append a timestamp.
    fn build_log_file_name(
        crate_name: &str,
        client_id: Option<&str>,
        now: &chrono::DateTime<chrono::Local>,
    ) -> String {
        match client_id.filter(|s| !s.is_empty()) {
            Some(id) => format!("{crate_name}-{id}.log"),
            None => format!("{crate_name}-{}.log", now.format("%Y%m%d_%H%M%S")),
        }
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
    fn test_load_base_plus_local_anvil_toml_config() {
        let _guard = TEST_MUTEX.lock().unwrap();

        // base.toml on its own is incomplete (no `environment`); we always overlay a profile.
        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("local-anvil".to_string()))
                .expect("Failed to load local-anvil config");

        assert_eq!(
            Some("0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c"),
            config.indexer.initial_block_hash.as_deref()
        );
        // local-anvil overrides start_from to Best (base sets Hash).
        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
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
    fn test_docker_anvil_environment_overrides() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("docker-anvil".to_string()))
                .expect("Failed to load config with docker-anvil environment");

        assert_eq!(
            Some("0xa3b056ebbb4ca08f79975bc9a1d53b4fc68b011b0480b2241f7c03543bc3d22c"),
            config.indexer.initial_block_hash.as_deref()
        );
        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert_eq!("/app/db/", config.indexer.storage.path); // override
        assert_eq!(1000, config.indexer.cache.size);
        // docker-anvil shares the anvil dev tier with local-anvil at runtime.
        assert_eq!("local-anvil", config.environment);
        assert_eq!("ws://host.docker.internal:8545", config.provider.rootstock.url);
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(10, config.contracts.len());
    }

    #[test]
    fn test_local_rskj_environment_overrides() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("local-rskj".to_string()))
                .expect("Failed to load config with local-rskj environment");

        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert_eq!("local-rskj", config.environment);
        assert_eq!("ws://127.0.0.1:8546", config.provider.rootstock.url);
        assert_eq!("regtest", config.bitcoin_network);
        // RSKj has its own [[contracts]] list (8 entries): no anvil-only
        // TestContract*, no rskj-only AccessManager/BitcoinManager/RbtcBridge.
        // Addresses come from the RSKj deploy where no BridgeMock is created,
        // so the deployer's nonce sequence is offset by one vs Anvil's predeploy.
        assert_eq!(8, config.contracts.len());
        let pegin = config.contracts.iter().find(|c| c.name == "PeginManager").unwrap();
        assert_eq!("0x959922bE3CAee4b8Cd9a407cc3ac1C251C2007B1", pegin.address);
        let stream = config.contracts.iter().find(|c| c.name == "StreamManager").unwrap();
        assert_eq!("0x5FC8d32690cc91D4c39d9d3abcBD16989F875707", stream.address);
        let native = config.contracts.iter().find(|c| c.name == "NativeBridge").unwrap();
        assert_eq!("0x0000000000000000000000000000000001000006", native.address);
    }

    #[test]
    fn test_docker_rskj_environment_overrides() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("docker-rskj".to_string()))
                .expect("Failed to load config with docker-rskj environment");

        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert_eq!("/app/db/", config.indexer.storage.path);
        // docker-rskj shares the rskj dev tier with local-rskj at runtime.
        assert_eq!("local-rskj", config.environment);
        assert_eq!("ws://host.docker.internal:8546", config.provider.rootstock.url);
        assert_eq!("regtest", config.bitcoin_network);
        // Same 8-entry rskj-specific [[contracts]] list as local-rskj.
        assert_eq!(8, config.contracts.len());
        let pegin = config.contracts.iter().find(|c| c.name == "PeginManager").unwrap();
        assert_eq!("0x959922bE3CAee4b8Cd9a407cc3ac1C251C2007B1", pegin.address);
    }

    #[test]
    fn test_local_anvil_environment_overrides() {
        let _guard = TEST_MUTEX.lock().unwrap();

        let config: CommonConfig =
            CommonConfig::load_config::<CommonConfig>(Some("local-anvil".to_string()))
                .expect("Failed to load config with local-anvil environment");

        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert_eq!("local-anvil", config.environment);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
    }

    #[test]
    fn test_base_alone_fails_to_load() {
        let _guard = TEST_MUTEX.lock().unwrap();

        // base.toml omits `environment` (required field). Loading without an overlay must error.
        assert!(CommonConfig::load_config::<CommonConfig>(None).is_err());
    }

    #[test]
    fn test_environment_variables_override_config_files() {
        let _guard = TEST_MUTEX.lock().unwrap();

        // SAFETY: Access to process-global env vars is serialized via TEST_MUTEX (held above).
        unsafe {
            set_var("UB__ENVIRONMENT", "local-anvil");
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

        // SAFETY: serialized via TEST_MUTEX.
        unsafe {
            remove_var("UB__ENVIRONMENT");
        }
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
            CommonConfig::load_config::<CommonConfig>(Some("docker-anvil".to_string()))
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

    #[test]
    fn test_resolve_log_dir_prefers_cli_arg() {
        assert_eq!("from_arg", CommonConfig::resolve_log_dir(Some("from_arg"), Some("from_env")));
    }

    #[test]
    fn test_resolve_log_dir_falls_back_to_env() {
        assert_eq!("from_env", CommonConfig::resolve_log_dir(None, Some("from_env")));
    }

    #[test]
    fn test_resolve_log_dir_uses_default_when_both_missing() {
        assert_eq!(DEFAULT_LOG_DIR, CommonConfig::resolve_log_dir(None, None));
    }

    #[test]
    fn test_resolve_log_dir_treats_empty_strings_as_unset() {
        // Empty arg falls through to env; empty env falls through to default.
        assert_eq!("from_env", CommonConfig::resolve_log_dir(Some(""), Some("from_env")));
        assert_eq!(DEFAULT_LOG_DIR, CommonConfig::resolve_log_dir(Some(""), Some("")));
    }

    #[test]
    fn test_select_json_format_log_format_overrides_environment() {
        // LOG_FORMAT=json wins regardless of ENVIRONMENT.
        assert!(CommonConfig::select_json_format(Some("json"), Some("local")));
        // LOG_FORMAT=pretty (or anything non-json) wins regardless of ENVIRONMENT.
        assert!(!CommonConfig::select_json_format(Some("pretty"), Some("staging")));
    }

    #[test]
    fn test_select_json_format_log_format_is_case_insensitive() {
        assert!(CommonConfig::select_json_format(Some("JSON"), None));
        assert!(CommonConfig::select_json_format(Some("Json"), None));
    }

    #[test]
    fn test_select_json_format_falls_back_to_environment() {
        // Non-local environment ⇒ JSON.
        assert!(CommonConfig::select_json_format(None, Some("docker")));
        assert!(CommonConfig::select_json_format(None, Some("staging")));
        // Local environment ⇒ pretty.
        assert!(!CommonConfig::select_json_format(None, Some("local")));
    }

    #[test]
    fn test_select_json_format_defaults_to_pretty_when_unset() {
        assert!(!CommonConfig::select_json_format(None, None));
    }

    #[test]
    fn test_select_json_format_treats_empty_strings_as_unset() {
        // Empty LOG_FORMAT falls through to ENVIRONMENT.
        assert!(CommonConfig::select_json_format(Some(""), Some("docker")));
        // Empty ENVIRONMENT falls through to default (pretty).
        assert!(!CommonConfig::select_json_format(None, Some("")));
    }

    #[test]
    fn test_build_log_file_name_with_client_id_uses_stable_name() {
        let now = chrono::Local::now();
        assert_eq!(
            "block-indexer-3.log",
            CommonConfig::build_log_file_name("block-indexer", Some("3"), &now)
        );
    }

    #[test]
    fn test_build_log_file_name_without_client_id_includes_timestamp() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-21T12:34:56Z")
            .unwrap()
            .with_timezone(&chrono::Local);
        let formatted = now.format("%Y%m%d_%H%M%S").to_string();
        let expected = format!("coordinator-{formatted}.log");
        assert_eq!(expected, CommonConfig::build_log_file_name("coordinator", None, &now));
    }

    #[test]
    fn test_build_log_file_name_empty_client_id_treated_as_none() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-21T12:34:56Z")
            .unwrap()
            .with_timezone(&chrono::Local);
        let formatted = now.format("%Y%m%d_%H%M%S").to_string();
        let expected = format!("user-api-{formatted}.log");
        // Empty CLIENT_ID should fall through to the timestamped path.
        assert_eq!(expected, CommonConfig::build_log_file_name("user-api", Some(""), &now));
    }
}
