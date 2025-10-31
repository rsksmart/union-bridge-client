use common::config::CommonConfig;
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub bitcoin_network: String,
    #[serde(rename = "user-api")]
    pub user_api_config: UserApiConfig,
}

#[derive(Debug, Deserialize)]
pub struct UserApiConfig {
    pub coordinator_broker_client_id: u32, // TODO(Jira) for now just one client ID until we unify the brokers in scope of https://rsklabs.atlassian.net/browse/UB-215
    pub broker_server_port: u16,
    pub http_server_port: u16,
}

impl Config {
    pub fn load(env_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(env_name)
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
    use common::config::CommonConfig;

    #[test]
    fn test_load_base_toml_config() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load base config");

        assert_eq!(1, config.user_api_config.coordinator_broker_client_id);
        assert_eq!(9007, config.user_api_config.broker_server_port);
        assert_eq!(8080, config.user_api_config.http_server_port);
    }
}
