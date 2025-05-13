use common::config::CommonConfig;
use common::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Deserialize)]
pub struct Config {
    pub log_broker_port: u16,
    pub block_broker_port: u16,
    pub broker_client_id: u32,
}

impl Config {
    pub fn load(base_path: Option<&String>) -> Result<Self, ConfigError> {
        let (cfg, _) = CommonConfig::load_config::<Self>(base_path, CARGO_PKG_NAME)?;
        Ok(cfg)
    }
}

pub struct Logger {}

impl Logger {
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}
