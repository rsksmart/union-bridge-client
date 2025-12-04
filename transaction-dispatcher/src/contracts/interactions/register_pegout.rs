use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterPegoutInput, RegisterPegoutOutput};
use anyhow::Result;
use log::info;
use union_contracts::bindings::peg_manager::PegManager::BtcTxSPVProof;

#[derive(Clone)]
pub(crate) struct RegisterPegoutInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> RegisterPegoutInvoke<C> {
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

        let tx_hash = self
            .contract
            .invoke_register_pegout(parsed_input, self.gas_bumps)
            .await?;

        info!("invoke_register_pegout successful at tx {tx_hash}");
        Ok(RegisterPegoutOutput {
            transaction_hash: tx_hash.to_string(),
        })
    }
}

//todo test
