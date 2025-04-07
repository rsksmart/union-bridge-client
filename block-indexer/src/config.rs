use common::config::{CommonConfig, IndexerConfig, ProviderConfig};
use common::errors::ConfigError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub indexer: IndexerConfig,
    pub provider: ProviderConfig,
}

impl Config {
    pub fn load(base_path: &str) -> Result<Self, ConfigError> {
        CommonConfig::load::<Self>(base_path, env!("CARGO_PKG_NAME"))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    #[test]
    fn test_config_load_when_stage_config_set_should_load_config_successfully() {
        let config_path = format!("{}/../config/stage", CARGO_MANIFEST_DIR);
        let config: Config = Config::load(&config_path).expect("Failed to load config");

        // indexer
        assert_eq!(
            "0xf6e292fd22f1dc5a1ef4022b7fe4a959f90ec0b9f5fc0869af64b99195511b22",
            config.indexer.initial_block_hash
        );
        assert_eq!(
            "/tmp/monitor-executions/default/storage",
            config.indexer.storage.path
        );
        assert_eq!(1000, config.indexer.cache.size);

        // provider
        assert_eq!(
            "ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket",
            config.provider.rootstock.url
        );
    }
}
