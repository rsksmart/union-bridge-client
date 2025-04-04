use crate::contracts::peg_manager::{PegManager, PegManagerAlloyWrapper, PegManagerInstance};
use alloy_primitives::Address;
use alloy_provider::RootProvider;
use anyhow::{Context, Result};
use common::{config::Config, types::ContractInfo};
use std::collections::HashMap;

/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

pub trait RskContractsGateway {
    type Instance: PegManagerInstance;
    fn get_peg_manager(&self) -> &PegManager<Self::Instance>;
}

pub struct RskContractsGatewayAlloy {
    peg_manager_contract: PegManager<PegManagerAlloyWrapper>,
}

impl RskContractsGatewayAlloy {
    pub fn new(provider: &RootProvider, config: &Config) -> Result<Self> {
        let contract_address = Self::load_contract(
            PEG_MANAGER_CONTRACT_NAME,
            config.load_managed_contracts(true),
        )?;
        let peg_manager_contract = PegManager::init(&provider, contract_address)
            .context("Could not instantiate PegManagerContract")?;
        Ok(RskContractsGatewayAlloy {
            peg_manager_contract,
        })
    }

    fn load_contract(name: &str, contracts: HashMap<String, ContractInfo>) -> Result<Address> {
        contracts
            .get(name)
            .context(format!("Address not found for contract: {}", name))?
            .address
            .to_string()
            .parse::<Address>()
            .context("Parsing to Address failed")
    }
}

impl RskContractsGateway for RskContractsGatewayAlloy {
    type Instance = PegManagerAlloyWrapper;
    fn get_peg_manager(&self) -> &PegManager<Self::Instance> {
        &self.peg_manager_contract
    }
}
