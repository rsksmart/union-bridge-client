use common::config::{CommonConfig, ContractConfig};
use common::errors::ConfigError;
use common::types::Address;
use serde::Deserialize;
use std::net::IpAddr;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const PEG_MANAGER_CONTRACT_NAME: &str = "PegManager";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub log_broker: BrokerConfig,
    pub block_broker: BrokerConfig,
    pub bitvmx_broker: BrokerConfig,
    pub broker_client_id: u32,
    pub contracts: Vec<ContractConfig>,
}

#[derive(Debug, Deserialize)]
pub struct BrokerConfig {
    pub ip: IpAddr,
    pub port: u16,
}

impl Config {
    pub fn load(base_path: Option<&String>) -> Result<Self, ConfigError> {
        let (cfg, _) = CommonConfig::load_config::<Self>(base_path, CARGO_PKG_NAME)?;
        Ok(cfg)
    }

    pub fn get_peg_manager_contract(&self) -> Address {
        self.contracts
            .iter()
            .find(|contract| contract.name == PEG_MANAGER_CONTRACT_NAME)
            .map(|contract| contract.address.clone())
            .map(|address| {
                Address::try_from(address.as_str()).expect("Invalid contract address on config")
            })
            .expect("PegManager contract not found on config")
    }
}

pub struct Logger {}

impl Logger {
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}
