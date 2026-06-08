use common::config::{CommonConfig, IndexerConfig, KeyStoreConfig, NotifierConfig, ProviderConfig};
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
    pub key_store: KeyStoreConfig,
    #[serde(rename = "block_indexer")]
    pub block_indexer_config: BlockIndexerConfig,
}
#[derive(Debug, Deserialize)]
pub struct BlockIndexerConfig {
    pub notifier: NotifierConfig,
    pub coordinator: CoordinatorConfig,
    pub broker_key_path: String,
}

#[derive(Debug, Deserialize)]
pub struct CoordinatorConfig {
    pub client_id: u32,
    #[serde(default)]
    pub pubkey_hash: String,
}

impl Config {
    /// Load configuration from file
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the configuration file cannot be loaded or parsed
    pub fn load(config_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(config_name)
    }
}

pub struct Logger {}

impl Logger {
    /// Initialize logger.
    ///
    /// `log_dir_opt` is an optional directory for log files. When `None`, the
    /// `UB_LOG_DIR` env var is consulted; if neither is set, logs are written
    /// under `./logs/` (relative to the current working directory).
    ///
    /// Returns a [`common::config::LogGuard`] that must be kept alive for the
    /// duration of the process to flush the background file-writer thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the log directory cannot be created, or if a global
    /// tracing subscriber has already been installed (e.g. in tests that call
    /// this more than once).
    pub fn init(log_dir_opt: Option<&String>) -> anyhow::Result<common::config::LogGuard> {
        CommonConfig::init_logger(log_dir_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use common::config::{CommonConfig, IndexerStartFrom};

    use crate::config::{CARGO_PKG_NAME, Config};

    #[test]
    fn test_config_load_when_custom_config_set_should_load_config_successfully() {
        // base.toml no longer carries [[contracts]] or `environment`; per-env
        // overlays do, so load with an explicit profile.
        let config: Config = CommonConfig::load_config::<Config>(Some("local-anvil".to_string()))
            .expect("Failed to load config");

        assert_eq!(10001, config.block_indexer_config.notifier.port);
        assert_eq!(101, config.block_indexer_config.coordinator.client_id);
        assert_eq!("<to_patch_with_env>", config.block_indexer_config.coordinator.pubkey_hash);
        // local-anvil overrides start_from to Best (base sets Hash).
        assert_eq!(IndexerStartFrom::Best, config.indexer.start_from);
        assert!(!config.indexer.storage.path.contains("{BASE_STORAGE_PATH}"));
        assert!(config.indexer.storage.path.ends_with("/.union_bridge/op_1/local_database"));
        assert_eq!(1000, config.indexer.cache.size);
        assert_eq!("ws://127.0.0.1:8545", config.provider.rootstock.url);
        assert!(
            config
                .block_indexer_config
                .broker_key_path
                .ends_with("/.union_bridge/op_1/union-client/broker/block-indexer.pem")
        );
    }

    #[test]
    fn test_init_logger() {
        // Smoke test: a different test in this binary may have already installed
        // a global subscriber, in which case init_logger legitimately Errs. We
        // only assert the call doesn't panic.
        let _ = CommonConfig::init_logger(None, CARGO_PKG_NAME);
    }
}
