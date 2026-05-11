use common::config::{CommonConfig, KeyStoreConfig};
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub bitcoin_network: String,
    pub key_store: KeyStoreConfig,
    #[serde(rename = "user_api")]
    pub user_api_config: UserApiConfig,
}

#[derive(Debug, Deserialize)]
pub struct UserApiConfig {
    /// Temporary coordinator routing config for user-api broker messages.
    /// Remove this section when user-api is removed.
    pub coordinator: CoordinatorConfig,
    pub broker_key_path: String,
    pub notifier: NotifierConfig,
    pub http: HttpConfig,
    /// Bearer token required by `POST /admin/fail-flow`. When unset/empty the endpoint returns 401.
    #[serde(default)]
    pub admin_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CoordinatorConfig {
    pub client_id: u32,
    #[serde(default)]
    pub pubkey_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct NotifierConfig {
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct HttpConfig {
    pub port: u16,
}

impl Config {
    pub fn load(config_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(config_name)
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
    use common::config::CommonConfig;

    use super::*;

    #[test]
    fn test_load_base_toml_config() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load base config");

        assert_eq!(101, config.user_api_config.coordinator.client_id);
        assert_eq!("<to_patch_with_env>", config.user_api_config.coordinator.pubkey_hash);
        assert!(config
            .user_api_config
            .broker_key_path
            .ends_with("/.union_bridge/op_1/union-client/broker/user-api.pem"));
        assert_eq!(30001, config.user_api_config.notifier.port);
        assert_eq!(40001, config.user_api_config.http.port);
    }
}
