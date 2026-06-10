use std::time::Duration;

use common_core::types::Address;
use common_runtime::config::{CommonConfig, ContractConfig, KeyStoreConfig};
use common_runtime::errors::ConfigError;
use serde::Deserialize;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const PEGIN_MANAGER_CONTRACT_NAME: &str = "PeginManager";
const PEGOUT_MANAGER_CONTRACT_NAME: &str = "PegoutManager";
const SIGNATURE_CONTRACT_NAME: &str = "SignatureManager";
const COMMITTEE_REGISTRY_CONTRACT_NAME: &str = "CommitteeRegistry";
const MEMBER_REGISTRY_CONTRACT_NAME: &str = "MemberRegistry";
const STREAM_MANAGER_CONTRACT_NAME: &str = "StreamManager";

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Runtime tier classification (`local-anvil`, `local-rskj`, ...). Required:
    /// every overlay must set this; base.toml is incomplete on its own.
    pub environment: String,
    /// `[[contracts]]` blocks live in the per-env overlay TOMLs, not in
    /// `base.toml`. Default to empty so base alone deserializes.
    #[serde(default)]
    pub contracts: Vec<ContractConfig>,
    pub bitcoin_network: String, // loaded from common.yaml
    pub key_store: KeyStoreConfig,
    #[serde(rename = "coordinator")]
    pub coordinator: CoordinatorConfig,
    /// Flow configuration. Required — `[flows.common]` has no Rust-side default,
    /// so any config that omits `[flows]` (or `[flows.common]`) fails to parse.
    pub flows: FlowsConfig,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "coordinator")]
pub struct CoordinatorConfig {
    pub logs: BrokerConfig,
    pub blocks: BrokerConfig,
    pub user: BrokerConfig,
    pub bitvmx: BitVmxBrokerConfig,
    pub broker: BrokerClientConfig,
    pub storage_path: String,
    pub check_period_secs: u64,
    pub bitvmx_not_responding_threshold_secs: u64,
    pub bitvmx_ping_after_silence_secs: u64,
}

impl CoordinatorConfig {
    #[must_use]
    pub fn check_period(&self) -> Duration {
        Duration::from_secs(self.check_period_secs)
    }

    #[must_use]
    pub fn bitvmx_not_responding_threshold(&self) -> Duration {
        Duration::from_secs(self.bitvmx_not_responding_threshold_secs)
    }

    #[must_use]
    pub fn bitvmx_ping_after_silence(&self) -> Duration {
        Duration::from_secs(self.bitvmx_ping_after_silence_secs)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BrokerClientConfig {
    pub client_id: u32,
    pub key_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub pubkey_hash: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitVmxBrokerConfig {
    pub host: String,
    pub port: u16,
    /// The `pubkey_hash` of the bitvmx broker server's message queue.
    /// This should match the `components.bitvmx.pubkey_hash` in the bitvmx-client config.
    pub pubkey_hash: String,
}

// ═══════════════════════════════════════════════════════
// Flow Configuration
// ═══════════════════════════════════════════════════════

/// Top-level flow configuration composing all flow-specific configs.
/// Loaded from TOML with serde, using 3-tier hierarchy: base.toml -> env.toml -> UB__ env vars.
#[derive(Debug, Clone, Deserialize)]
pub struct FlowsConfig {
    /// Common flow settings (required — no Rust-side default).
    pub common: CommonFlowConfig,
    /// Pegout flow settings.
    #[serde(default)]
    pub pegout: PegoutConfig,
    /// Committee setup settings.
    #[serde(default)]
    pub committee: CommitteeConfig,
    /// Native bridge verification settings.
    #[serde(default)]
    pub native_bridge: NativeBridgeConfig,
}

/// Common flow configuration. All fields are required in the runtime config —
/// there is no Rust-side `Default`, so a missing field is a parse error rather
/// than a silent fall-back to a hardcoded value.
#[derive(Debug, Clone, Deserialize)]
pub struct CommonFlowConfig {
    /// Required RSK block confirmations for events.
    pub rsk_confirmations: u32,
    /// BTC confirmations used by non-Native-Bridge coordinator flows.
    pub btc_confirmations: u32,
    /// Blocks delay before rechecking BTC transaction status.
    pub btc_status_retry_blocks: u32,
}

/// Pegout flow configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PegoutConfig {
    /// Timeout in seconds for advance funds before triggering operator take (default: 600 = 10 min)
    pub advance_funds_timeout_secs: u64,
}

impl Default for PegoutConfig {
    fn default() -> Self {
        Self { advance_funds_timeout_secs: 600 }
    }
}

impl PegoutConfig {
    /// Returns `advance_funds_timeout` as Duration
    #[must_use]
    pub fn advance_funds_timeout(&self) -> Duration {
        Duration::from_secs(self.advance_funds_timeout_secs)
    }
}

/// Committee setup flow configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CommitteeConfig {
    /// Path to the DRP program definition YAML file
    pub drp_program_definition: String,
}

/// Native bridge verification configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NativeBridgeConfig {
    /// Extra confirmations required on top of `coordinator.btc_confirmations` (default: 1)
    pub btc_confirmations_buffer: u32,
}

impl Default for NativeBridgeConfig {
    fn default() -> Self {
        Self { btc_confirmations_buffer: 1 }
    }
}

impl Config {
    /// # Errors
    /// Returns an error if configuration loading fails.
    pub fn load(config_name: Option<&str>) -> Result<Self, ConfigError> {
        CommonConfig::load_config::<Self>(config_name.map(str::to_string))
    }

    /// # Panics
    /// Panics if a contract address in the configuration is invalid.
    #[must_use]
    pub fn get_contract_addresses(&self) -> Vec<Address> {
        self.contracts
            .iter()
            .filter(|contract| Config::get_contracts_to_subscribe_to(contract))
            .map(|contract| contract.address.clone())
            .map(|address| {
                Address::try_from(address.as_str()).expect("Invalid contract address on config")
            })
            .collect::<Vec<Address>>()
    }

    fn get_contracts_to_subscribe_to(contract: &ContractConfig) -> bool {
        contract.name == PEGIN_MANAGER_CONTRACT_NAME
            || contract.name == PEGOUT_MANAGER_CONTRACT_NAME
            || contract.name == SIGNATURE_CONTRACT_NAME
            || contract.name == COMMITTEE_REGISTRY_CONTRACT_NAME
            || contract.name == MEMBER_REGISTRY_CONTRACT_NAME
            || contract.name == STREAM_MANAGER_CONTRACT_NAME
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
    /// Returns a [`common_runtime::logging::LogGuard`] that must be kept alive for the
    /// duration of the process to flush the background file-writer thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the log directory cannot be created, or if a global
    /// tracing subscriber has already been installed (e.g. in tests that call
    /// this more than once).
    pub fn init(log_dir_opt: Option<&String>) -> anyhow::Result<common_runtime::logging::LogGuard> {
        CommonConfig::init_logger(log_dir_opt, CARGO_PKG_NAME)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bitcoin::Network;
    use common_core::types::Address;
    use common_runtime::config::CommonConfig;

    use crate::config::Config;

    #[test]
    fn test_parse_bitcoin_network() -> anyhow::Result<()> {
        let config = CommonConfig::load_config::<Config>(Some("local-anvil".to_string()))?;
        assert_eq!(Network::Regtest, CommonConfig::parse_bitcoin_network(&config.bitcoin_network)?);
        Ok(())
    }

    #[test]
    fn test_load_local_anvil_toml_config() {
        let config: Config =
            Config::load(Some("local-anvil")).expect("Failed to load local-anvil config");

        assert_eq!("local-anvil", config.environment);
        assert_eq!("0.0.0.0", config.coordinator.logs.host);
        assert_eq!(20001, config.coordinator.logs.port);
        assert_eq!("<to_patch_with_env>", config.coordinator.logs.pubkey_hash);
        assert_eq!("0.0.0.0", config.coordinator.blocks.host);
        assert_eq!(10001, config.coordinator.blocks.port);
        assert_eq!("<to_patch_with_env>", config.coordinator.blocks.pubkey_hash);
        assert_eq!("0.0.0.0", config.coordinator.user.host);
        assert_eq!(30001, config.coordinator.user.port);
        assert_eq!("<to_patch_with_env>", config.coordinator.user.pubkey_hash);
        assert_eq!("0.0.0.0", config.coordinator.bitvmx.host);
        assert_eq!(22222, config.coordinator.bitvmx.port);
        assert_eq!(
            "1d10fa43ebbf6674d74caa3e9032711ade09d98ea7d20f89459f61152bebda1e",
            config.coordinator.bitvmx.pubkey_hash
        );
        assert_eq!(101, config.coordinator.broker.client_id);
        assert!(
            config
                .coordinator
                .broker
                .key_path
                .ends_with("/.union_bridge/op_1/union-client/broker/coordinator.pem")
        );
        assert!(!config.coordinator.storage_path.contains("{BASE_STORAGE_PATH}"));
        assert!(config.coordinator.storage_path.ends_with("/.union_bridge/op_1/local_database"));
        assert_eq!(config.coordinator.check_period_secs, 1);
        assert_eq!(config.coordinator.bitvmx_not_responding_threshold_secs, 30);
        assert_eq!(config.coordinator.bitvmx_ping_after_silence_secs, 15);
        // local-anvil overlay points DRP at the Docker mount path; base uses the repo-relative one.
        assert_eq!(
            "/app/resources/union-verifier.yaml",
            config.flows.committee.drp_program_definition
        );
        assert_eq!("regtest", config.bitcoin_network);
        assert_eq!(10, config.contracts.len());
    }

    #[test]
    fn test_coordinator_config_duration_helpers() {
        let config = Config::load(Some("local-anvil")).expect("Failed to load local-anvil config");
        let config = config.coordinator;
        assert_eq!(config.check_period(), Duration::from_secs(1));
        assert_eq!(config.bitvmx_not_responding_threshold(), Duration::from_secs(30));
        assert_eq!(config.bitvmx_ping_after_silence(), Duration::from_secs(15));
    }

    /// Guards the strict-config behavior: with `CommonFlowConfig` carrying no
    /// `Default` impl and `Config.flows` no longer marked `#[serde(default)]`,
    /// a config that omits `[flows]` must fail to parse rather than silently
    /// fall back to hardcoded Rust defaults.
    #[test]
    fn test_config_with_missing_flows_section_errors() {
        let result: Result<Config, _> = serde_json::from_value(serde_json::json!({
            "environment": "local-anvil",
            "contracts": [],
            "bitcoin_network": "regtest",
            "key_store": {
                "user_path": "/tmp/user.json",
                "member_path": "/tmp/member.json"
            },
            "coordinator": {
                "logs": { "host": "0.0.0.0", "port": 20001 },
                "blocks": { "host": "0.0.0.0", "port": 10001 },
                "user": { "host": "0.0.0.0", "port": 30001 },
                "bitvmx": {
                    "host": "0.0.0.0",
                    "port": 22222,
                    "pubkey_hash": "abc"
                },
                "broker": {
                    "client_id": 101,
                    "key_path": "/tmp/coordinator.pem"
                },
                "storage_path": "/tmp/coordinator",
                "check_period_secs": 1,
                "bitvmx_not_responding_threshold_secs": 30,
                "bitvmx_ping_after_silence_secs": 15
            }
        }));

        assert!(
            result.is_err(),
            "config without [flows] should fail to deserialize, got: {result:?}",
        );
    }

    #[test]
    fn test_get_contract_addresses_returns_runtime_subscriptions() {
        let config = Config::load(Some("local-anvil")).expect("Failed to load local-anvil config");
        let contract_addresses = config.get_contract_addresses();

        assert_eq!(6, contract_addresses.len());
        assert!(contract_addresses.iter().any(|address| {
            *address
                == Address::try_from("0x9A9f2CCfdE556A7E9Ff0848998Aa4a0CFD8863AE")
                    .expect("valid pegin manager address")
        }));
    }
}
