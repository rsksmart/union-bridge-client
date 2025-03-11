use crate::contracts::peg_manager::{PegManager, PegManagerAlloyWrapper, PegManagerErrors};
use crate::types::{PeginAddressInput, PeginAddressOutput};
use alloy_primitives::Address;
use alloy_provider::RootProvider;
use anyhow::{Context, Result};
use common::config::Config;
use common::types::ContractInfo;
use std::collections::HashMap;

pub struct RskContractsGateway {
    peg_manager_contract: PegManager<PegManagerAlloyWrapper>,
}

// TODO(iago) add a "managed_contracts" entry in transaction-dispatcher config and remove this hardcoding
/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

impl RskContractsGateway {
    pub fn new(provider: &RootProvider, config: &Config) -> Result<Self> {
        let contract =
            Self::load_contract(PEG_MANAGER_CONTRACT_NAME, config.load_managed_contracts())?;
        let peg_manager_contract = PegManager::init(&provider, contract)
            .context("Could not instantiate PegManagerContract")?;
        Ok(RskContractsGateway {
            peg_manager_contract,
        })
    }

    pub async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, PegManagerErrors> {
        self.peg_manager_contract
            .get_temporary_pegin_address(input)
            .await
    }

    fn load_contract(name: &str, contracts: HashMap<String, ContractInfo>) -> Result<Address> {
        contracts
            .get(name)
            .context(format!("Address not found for contract: {}", name))?
            .address
            .parse()
            .context(format!("Could not parse contract address for: {}", name))
    }
}
