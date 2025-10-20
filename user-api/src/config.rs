use common::config::CommonConfig;
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub coordinator_broker_client_id: u32, // TODO(Jira) for now just one client ID until we unify the brokers in scope of https://rsklabs.atlassian.net/browse/UB-215
    pub broker_server_port: u16,
    pub http_server_port: u16,
    pub bitcoin_network: String, // loaded from common.yaml
}

impl Config {
    pub fn load(env_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config_2::<Self>(env_name)
    }
}

pub struct Logger {}

impl Logger {
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}
