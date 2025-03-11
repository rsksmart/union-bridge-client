use crate::contracts::peg_manager::{PegManager, PegManagerAlloyWrapper, PegManagerErrors};
use crate::types::{PeginAddressInput, PeginAddressOutput};
use alloy_primitives::Address;
use alloy_provider::RootProvider;
use anyhow::{Context, Result};
use common::config::Config;
use common::types::ContractInfo;
use std::collections::HashMap;

// TODO(iago) add a "managed_contracts" entry in transaction-dispatcher config and remove this hardcoding
/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

pub trait RskContractsGateway {
    #[allow(async_fn_in_trait)]
    fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> impl Future<Output = Result<PeginAddressOutput, PegManagerErrors>> + Send;
}

pub struct RskContractsGatewayAlloy {
    peg_manager_contract: PegManager<PegManagerAlloyWrapper>,
}

impl RskContractsGatewayAlloy {
    pub fn new(provider: &RootProvider, config: &Config) -> Result<Self> {
        let contract =
            Self::load_contract(PEG_MANAGER_CONTRACT_NAME, config.load_managed_contracts())?;
        let peg_manager_contract = PegManager::init(&provider, contract)
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
            .parse()
            .context(format!("Could not parse contract address for: {}", name))
    }
}

impl RskContractsGateway for RskContractsGatewayAlloy {
    async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, PegManagerErrors> {
        self.peg_manager_contract
            .get_temporary_pegin_address(input)
            .await
    }
}
