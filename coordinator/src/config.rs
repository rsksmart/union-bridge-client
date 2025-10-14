use common::config::{CommonConfig, ContractConfig};
use common::errors::ConfigError;
use common::types::Address;
use serde::Deserialize;

// TODO this should be event-type-dependent, therefore for now we use a constant - it makes no sense adding it to the config
pub const REQUIRED_CONFIRMATIONS: u32 = 5;
pub const SPV_PROOF_MIN_CONFIRMATIONS: u32 = 10;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const PEG_MANAGER_CONTRACT_NAME: &str = "PegManager";
const SIGNATURE_CONTRACT_NAME: &str = "SignatureManager";
const COMMITTEE_REGISTRY_CONTRACT_NAME: &str = "CommitteeRegistry";
const MEMBER_REGISTRY_CONTRACT_NAME: &str = "MemberRegistry";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub log_broker: BrokerConfig,
    pub block_broker: BrokerConfig,
    pub user_broker: BrokerConfig,
    pub bitvmx_broker: BrokerConfig,
    pub broker_client_id: u32,
    pub contracts: Vec<ContractConfig>,
    pub storage_path: String,
    pub bitcoin_network: String, // loaded from common.yaml
}

#[derive(Debug, Deserialize)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn load(base_path: Option<&String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(base_path, CARGO_PKG_NAME)
    }

    pub fn get_contract_addresses(&self) -> Vec<Address> {
        self.contracts
            .iter()
            .filter(|contract| Self::get_contracts_to_subscribe_to(contract))
            .map(|contract| contract.address.clone())
            .map(|address| {
                Address::try_from(address.as_str()).expect("Invalid contract address on config")
            })
            .collect::<Vec<Address>>()
    }

    #[cfg(feature = "anvil")]
    fn get_contracts_to_subscribe_to(contract: &ContractConfig) -> bool {
        contract.name == PEG_MANAGER_CONTRACT_NAME
            || contract.name == "FakePegManager"
            || contract.name == SIGNATURE_CONTRACT_NAME
            || contract.name == COMMITTEE_REGISTRY_CONTRACT_NAME
            || contract.name == MEMBER_REGISTRY_CONTRACT_NAME
    }

    #[cfg(not(feature = "anvil"))]
    fn get_contracts_to_subscribe_to(contract: &ContractConfig) -> bool {
        contract.name == PEG_MANAGER_CONTRACT_NAME
            || contract.name == SIGNATURE_CONTRACT_NAME
            || contract.name == COMMITTEE_REGISTRY_CONTRACT_NAME
            || contract.name == MEMBER_REGISTRY_CONTRACT_NAME
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
    use crate::config::Config;
    use bitcoin::Network;
    use common::config::CommonConfig;

    #[test]
    fn test_parse_bitcoin_network() -> anyhow::Result<()> {
        const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

        let config_path = &format!("{}/../config/multi-client-template", CARGO_MANIFEST_DIR);
        let config = Config::load(Some(config_path))?;
        assert_eq!(
            Network::Regtest,
            CommonConfig::parse_bitcoin_network(&config.bitcoin_network)?
        );
        Ok(())
    }
}
