use common::config::{CommonConfig, ContractConfig, KeyStoreConfig};
use common::errors::ConfigError;
use common::types::Address;
use serde::Deserialize;

// TODO this should be event-type-dependent, therefore for now we use a constant - it makes no sense adding it to the config
pub const REQUIRED_CONFIRMATIONS: u32 = 5;
const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const PEG_MANAGER_CONTRACT_NAME: &str = "PegManager";
const SIGNATURE_CONTRACT_NAME: &str = "SignatureManager";
const COMMITTEE_REGISTRY_CONTRACT_NAME: &str = "CommitteeRegistry";
const MEMBER_REGISTRY_CONTRACT_NAME: &str = "MemberRegistry";
const STREAM_MANAGER_CONTRACT_NAME: &str = "StreamManager";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub contracts: Vec<ContractConfig>,
    pub bitcoin_network: String, // loaded from common.yaml
    pub key_store: KeyStoreConfig,
    #[serde(rename = "coordinator")]
    pub coordinator: CoordinatorConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "coordinator")]
pub struct CoordinatorConfig {
    pub logs: BrokerConfig,
    pub blocks: BrokerConfig,
    pub user: BrokerConfig,
    pub bitvmx: BitVmxBrokerConfig,
    pub broker: BrokerClientConfig,
    pub storage_path: String,
}

#[derive(Debug, Deserialize)]
pub struct BrokerClientConfig {
    pub client_id: u32,
}

#[derive(Debug, Deserialize)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct BitVmxBrokerConfig {
    pub host: String,
    pub port: u16,
    /// The pubkey_hash of the bitvmx broker server's message queue.
    /// This should match the `components.bitvmx.pubkey_hash` in the bitvmx-client config.
    pub pubkey_hash: String,
}

impl Config {
    /// # Errors
    /// Returns an error if configuration loading fails.
    pub fn load(env_name: Option<String>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(env_name)
    }

    /// # Panics
    /// Panics if a contract address in the configuration is invalid.
    #[must_use]
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
            || contract.name == STREAM_MANAGER_CONTRACT_NAME
    }

    #[cfg(not(feature = "anvil"))]
    fn get_contracts_to_subscribe_to(contract: &ContractConfig) -> bool {
        contract.name == PEG_MANAGER_CONTRACT_NAME
            || contract.name == SIGNATURE_CONTRACT_NAME
            || contract.name == COMMITTEE_REGISTRY_CONTRACT_NAME
            || contract.name == MEMBER_REGISTRY_CONTRACT_NAME
            || contract.name == SIGNATURE_CONTRACT_NAME
            || contract.name == STREAM_MANAGER_CONTRACT_NAME
    }
}

pub struct Logger {}

impl Logger {
    /// # Errors
    /// Returns an error if logger initialization fails.
    pub fn init(logger_file_opt: Option<&String>) -> anyhow::Result<()> {
        CommonConfig::init_logger(logger_file_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use common::config::CommonConfig;

    use crate::config::Config;

    #[test]
    fn test_parse_bitcoin_network() -> anyhow::Result<()> {
        let config = CommonConfig::load_config::<Config>(None)?;
        assert_eq!(Network::Regtest, CommonConfig::parse_bitcoin_network(&config.bitcoin_network)?);
        Ok(())
    }

    #[test]
    fn test_load_base_toml_config() {
        let config: Config =
            CommonConfig::load_config::<Config>(None).expect("Failed to load base config");

        assert_eq!("0.0.0.0", config.coordinator.logs.host);
        assert_eq!(20001, config.coordinator.logs.port);
        assert_eq!("0.0.0.0", config.coordinator.blocks.host);
        assert_eq!(10001, config.coordinator.blocks.port);
        assert_eq!("0.0.0.0", config.coordinator.user.host);
        assert_eq!(30001, config.coordinator.user.port);
        assert_eq!("0.0.0.0", config.coordinator.bitvmx.host);
        assert_eq!(22222, config.coordinator.bitvmx.port);
        assert_eq!(
            "1d10fa43ebbf6674d74caa3e9032711ade09d98ea7d20f89459f61152bebda1e",
            config.coordinator.bitvmx.pubkey_hash
        );
        assert_eq!(101, config.coordinator.broker.client_id);
        assert!(!config.coordinator.storage_path.contains("{BASE_STORAGE_PATH}"));
        assert!(
            config.coordinator.storage_path.ends_with("/.union_bridge/database/multi-client-1")
        );
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(8, config.contracts.len());
    }
}
