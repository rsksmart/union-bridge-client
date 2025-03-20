use crate::contracts::peg_manager::{ContractApi, ContractWrapper, PegManagerGateway};
use alloy_primitives::Address;
use alloy_provider::network::EthereumWallet;
use alloy_provider::Provider;
use anyhow::{Context, Result};
use common::{config::Config, types::ContractInfo};
use std::collections::HashMap;

/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

pub trait RskContractsGateway<P: Provider> {
    type Instance: ContractApi;
    fn get_peg_manager(&self) -> &PegManagerGateway<Self::Instance>;
}

pub struct RskContractsGatewayAlloy<P: Provider> {
    peg_manager_contract: PegManagerGateway<ContractWrapper<P>>,
}

impl<P: Provider> RskContractsGatewayAlloy<P> {
    pub fn new(provider: P, signer: EthereumWallet, config: &Config) -> Result<Self> {
        let contract_address = Self::load_contract(
            PEG_MANAGER_CONTRACT_NAME,
            config.load_managed_contracts(true),
        )?;
        let peg_manager_contract = PegManagerGateway::init(provider, signer, contract_address)
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

impl<P: Provider> RskContractsGateway<P> for RskContractsGatewayAlloy<P> {
    type Instance = ContractWrapper<P>;
    fn get_peg_manager(&self) -> &PegManagerGateway<Self::Instance> {
        &self.peg_manager_contract
    }
}
