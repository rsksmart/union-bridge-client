use anyhow::Result;
use log::info;

use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::DomainErrors;

#[derive(Clone)]
pub struct NotifyCheckForkCompleteInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> NotifyCheckForkCompleteInvoke<C> {
    pub fn new(contract: C, gas_bumps: u8) -> Self {
        Self { contract, gas_bumps }
    }

    pub async fn run(
        &self,
        input: &str, // TODO proper type for input
    ) -> Result<(), DomainErrors> {
        info!("Init NotifyCheckForkComplete for: {input:?}");

        let tx_hash = self.contract.notify_check_fork_completion(input, self.gas_bumps).await?;

        info!("NotifyCheckForkComplete successful at tx {tx_hash}");
        Ok(())
    }
}
