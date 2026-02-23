use std::time::Duration;

use common::config::{CommonConfig, ContractConfig};
use common::errors::ConfigError;
use common::types::Address;
use serde::Deserialize;

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
    #[serde(rename = "coordinator")]
    pub coordinator: CoordinatorConfig,
    /// Bridge flow configuration with sensible defaults
    #[serde(default)]
    pub bridge: BridgeConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "coordinator")]
pub struct CoordinatorConfig {
    pub logs: BrokerConfig,
    pub blocks: BrokerConfig,
    pub user: BrokerConfig,
    pub bitvmx: BrokerConfig,
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

// ═══════════════════════════════════════════════════════
// Bridge Flow Configuration
// ═══════════════════════════════════════════════════════

/// Top-level bridge configuration composing all flow-specific configs.
/// Loaded from TOML with serde, using 3-tier hierarchy: base.toml -> env.toml -> UB__ env vars.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BridgeConfig {
    /// Global coordinator settings
    pub coordinator: CoordinatorFlowConfig,
    /// Pegin flow settings
    pub pegin: PeginConfig,
    /// Pegout flow settings
    pub pegout: PegoutConfig,
    /// Operator take / advance funds settings
    pub advance_funds: AdvanceFundsConfig,
    /// Committee setup settings
    pub committee: CommitteeConfig,
    /// Native bridge verification settings
    pub native_bridge: NativeBridgeConfig,
}

/// Coordinator-level flow configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CoordinatorFlowConfig {
    /// Required RSK block confirmations for events (default: 5)
    pub required_confirmations: u32,
    /// Period between coordinator check cycles in seconds (default: 1)
    pub check_period_secs: u64,
    /// Threshold in seconds before considering `BitVMX` not responding (default: 30)
    pub bitvmx_not_responding_threshold_secs: u64,
    /// Seconds of silence before sending a ping to `BitVMX` (default: 15)
    pub bitvmx_ping_after_silence_secs: u64,
}

impl Default for CoordinatorFlowConfig {
    fn default() -> Self {
        Self {
            required_confirmations: 5,
            check_period_secs: 1,
            bitvmx_not_responding_threshold_secs: 30,
            bitvmx_ping_after_silence_secs: 15,
        }
    }
}

impl CoordinatorFlowConfig {
    /// Returns `check_period` as Duration
    #[must_use]
    pub fn check_period(&self) -> Duration {
        Duration::from_secs(self.check_period_secs)
    }

    /// Returns `bitvmx_not_responding_threshold` as Duration
    #[must_use]
    pub fn bitvmx_not_responding_threshold(&self) -> Duration {
        Duration::from_secs(self.bitvmx_not_responding_threshold_secs)
    }

    /// Returns `bitvmx_ping_after_silence` as Duration
    #[must_use]
    pub fn bitvmx_ping_after_silence(&self) -> Duration {
        Duration::from_secs(self.bitvmx_ping_after_silence_secs)
    }
}

/// Pegin flow configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PeginConfig {
    /// Minimum BTC transaction confirmations for pegin (default: 1)
    pub min_tx_confirmations: u32,
    /// Blocks delay before rechecking transaction status (default: 20)
    pub blocks_delay_for_tx_check: u32,
}

impl Default for PeginConfig {
    fn default() -> Self {
        Self { min_tx_confirmations: 1, blocks_delay_for_tx_check: 20 }
    }
}

/// Pegout flow configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PegoutConfig {
    /// Blocks delay before rechecking transaction status (default: 20)
    pub blocks_delay_for_tx_check: u32,
    /// Minimum SPV proof confirmations (default: 1)
    pub spv_proof_min_confirmations: u32,
    /// Timeout in seconds for advance funds before triggering operator take (default: 600 = 10 min)
    pub advance_funds_timeout_secs: u64,
}

impl Default for PegoutConfig {
    fn default() -> Self {
        Self {
            blocks_delay_for_tx_check: 20,
            spv_proof_min_confirmations: 1,
            advance_funds_timeout_secs: 600,
        }
    }
}

impl PegoutConfig {
    /// Returns `advance_funds_timeout` as Duration
    #[must_use]
    pub fn advance_funds_timeout(&self) -> Duration {
        Duration::from_secs(self.advance_funds_timeout_secs)
    }
}

/// Advance funds / operator take flow configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AdvanceFundsConfig {
    /// Minimum SPV proof confirmations for advance funds (default: 1)
    pub spv_proof_min_confirmations: u32,
    /// Blocks delay before rechecking transaction status (default: 20)
    pub blocks_delay_for_tx_check: u32,
}

impl Default for AdvanceFundsConfig {
    fn default() -> Self {
        Self { spv_proof_min_confirmations: 1, blocks_delay_for_tx_check: 20 }
    }
}

/// Committee setup flow configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CommitteeConfig {
    /// Minimum `BitVMX` funding balance in satoshis (default: `20_002_000`)
    pub min_funding_balance: u64,
    /// Minimum RSK balance in wei (default: `1_000_000_000_500_000` = ~1 RBTC + fees)
    pub min_rsk_balance: u64,
}

impl Default for CommitteeConfig {
    fn default() -> Self {
        Self { min_funding_balance: 20_002_000, min_rsk_balance: 1_000_000_000_500_000 }
    }
}

/// Native bridge verification configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NativeBridgeConfig {
    /// Minimum confirmations required from native bridge (default: 2)
    /// Note: This is 1 + 1 in the original code comment
    pub min_tx_confirmations: u32,
}

impl Default for NativeBridgeConfig {
    fn default() -> Self {
        Self {
            min_tx_confirmations: 2, // 1 + 1 as per original
        }
    }
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
    use std::time::Duration;

    use bitcoin::Network;
    use common::config::CommonConfig;

    use crate::config::{BridgeConfig, Config, CoordinatorFlowConfig};

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
        assert_eq!(101, config.coordinator.broker.client_id);
        assert!(!config.coordinator.storage_path.contains("{BASE_STORAGE_PATH}"));
        assert!(
            config.coordinator.storage_path.ends_with("/.union_bridge/database/multi-client-1")
        );
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(9, config.contracts.len());
    }

    #[test]
    fn test_bridge_config_defaults_match_hardcoded_values() {
        let config = BridgeConfig::default();

        // Coordinator defaults (was REQUIRED_CONFIRMATIONS = 5, CHECK_PERIOD = 1s, etc.)
        assert_eq!(config.coordinator.required_confirmations, 5);
        assert_eq!(config.coordinator.check_period_secs, 1);
        assert_eq!(config.coordinator.bitvmx_not_responding_threshold_secs, 30);
        assert_eq!(config.coordinator.bitvmx_ping_after_silence_secs, 15);

        // Pegin defaults (was MIN_TX_CONFIRMATIONS = 1, BLOCKS_DELAY_FOR_TX_CHECK = 20)
        assert_eq!(config.pegin.min_tx_confirmations, 1);
        assert_eq!(config.pegin.blocks_delay_for_tx_check, 20);

        // Pegout defaults
        assert_eq!(config.pegout.blocks_delay_for_tx_check, 20);
        assert_eq!(config.pegout.spv_proof_min_confirmations, 1);
        assert_eq!(config.pegout.advance_funds_timeout_secs, 600);

        // Advance funds defaults
        assert_eq!(config.advance_funds.spv_proof_min_confirmations, 1);
        assert_eq!(config.advance_funds.blocks_delay_for_tx_check, 20);

        // Committee defaults (was MIN_FUNDING_BALANCE = 20_002_000, MIN_RSK_BALANCE = 100_000 * 10^10 + 500_000)
        assert_eq!(config.committee.min_funding_balance, 20_002_000);
        assert_eq!(config.committee.min_rsk_balance, 1_000_000_000_500_000);

        // Native bridge defaults (was MIN_TX_CONFIRMATIONS = 2)
        assert_eq!(config.native_bridge.min_tx_confirmations, 2);
    }

    #[test]
    fn test_bridge_config_duration_helpers() {
        let config = CoordinatorFlowConfig::default();
        assert_eq!(config.check_period(), Duration::from_secs(1));
        assert_eq!(config.bitvmx_not_responding_threshold(), Duration::from_secs(30));
        assert_eq!(config.bitvmx_ping_after_silence(), Duration::from_secs(15));
    }

    #[test]
    fn test_config_with_missing_bridge_section_uses_defaults() {
        // When loading existing config files without [bridge] section, defaults should be used
        let config = Config::load(None).expect("Should load with defaults");

        // Verify bridge config uses defaults
        assert_eq!(config.bridge.coordinator.required_confirmations, 5);
        assert_eq!(config.bridge.pegin.min_tx_confirmations, 1);
        assert_eq!(config.bridge.pegout.spv_proof_min_confirmations, 1);
    }
}
