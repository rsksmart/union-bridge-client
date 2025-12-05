use crate::contracts::pegout_manager::PegoutManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterPegoutInput, RegisterPegoutOutput};
use anyhow::Result;
use log::{error, info};
use union_contracts::bindings::pegout_manager::PegoutManager::BtcTxSPVProof;

#[derive(Clone)]
pub(crate) struct RegisterPegoutInvoke<C: PegoutManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegoutManagerContractApi> RegisterPegoutInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterPegoutInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterPegoutInput,
    ) -> Result<RegisterPegoutOutput, DomainErrors> {
        info!("Init RegisterPegout for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!("Failed to parse RegisterPegoutInput: {e}"))
        })?;

        let receipt = self
            .contract
            .invoke_register_user_take(parsed_input, self.gas_bumps)
            .await?;

        let tx_hash = receipt.transaction_hash();
        info!("invoke_register_pegout successful at tx {tx_hash}");
        Ok(RegisterPegoutOutput {
            transaction_hash: tx_hash.to_string(),
        })
    }
}

//todo test
