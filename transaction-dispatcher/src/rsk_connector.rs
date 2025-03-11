use crate::contracts::peg_manager::{PegManagerContract, PegManagerContractErrors};
use crate::types::{BaseContract, PeginAddressInput, PeginAddressOutput};
use alloy_provider::RootProvider;
use anyhow::{Context, Result};
use common::config::Config;

pub struct RskContractsGateway {
    peg_manager_contract: PegManagerContract,
}

impl RskContractsGateway {
    pub fn new(provider: &RootProvider, config: &Config) -> Result<Self> {
        let peg_manager_contract =
            PegManagerContract::new(&provider, config.load_managed_contracts())
                .context("Could not instantiate PegManagerContract")?;
        Ok(RskContractsGateway {
            peg_manager_contract,
        })
    }

    pub async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, PegManagerContractErrors> {
        self.peg_manager_contract
            .get_temporary_pegin_address(input)
            .await
    }
}
