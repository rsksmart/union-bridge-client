use crate::contracts::peg_manager::{PegManagerContract, PegManagerContractApi, PegManagerGateway};
use alloy_primitives::Address;
use alloy_provider::network::EthereumWallet;
use alloy_provider::Provider;
use anyhow::{Context, Result};
use common::{config::Config, types::ContractInfo};
use std::collections::HashMap;

/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

pub trait RskContractsGatewayApi<P: Provider> {
    type Instance: PegManagerContractApi;
    fn get_peg_manager(&self) -> &PegManagerGateway<Self::Instance>;
}

pub struct RskContractsGateway<P: Provider> {
    peg_manager: PegManagerGateway<PegManagerContract<P>>,
}

impl<P: Provider> RskContractsGateway<P> {
    pub fn new(provider: P, signer: EthereumWallet, config: &Config) -> Result<Self> {
        let contract_address = Self::load_contract(
            PEG_MANAGER_CONTRACT_NAME,
            config.load_managed_contracts(true),
        )?;
        let peg_manager = PegManagerGateway::init(provider, signer, contract_address)
            .context("Could not instantiate PegManagerContract")?;
        Ok(RskContractsGateway { peg_manager })
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

impl<P: Provider> RskContractsGatewayApi<P> for RskContractsGateway<P> {
    type Instance = PegManagerContract<P>;
    fn get_peg_manager(&self) -> &PegManagerGateway<Self::Instance> {
        &self.peg_manager
    }
}
