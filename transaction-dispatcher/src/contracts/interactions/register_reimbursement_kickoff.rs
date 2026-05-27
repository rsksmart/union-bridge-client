use tracing::info;
use union_contracts::bindings::pegout_manager::PegoutManager::BtcTxSPVProof;

use crate::contracts::pegout_manager::PegoutManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterReimbursementKickoffInput, RegisterReimbursementKickoffOutput};

#[derive(Clone)]
pub(crate) struct RegisterReimbursementKickoffInvoke<C: PegoutManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegoutManagerContractApi> RegisterReimbursementKickoffInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterReimbursementKickoffInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterReimbursementKickoffInput,
    ) -> Result<RegisterReimbursementKickoffOutput, DomainErrors> {
        info!("Init RegisterReimbursementKickoff for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.kickoff_spv_proof.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!(
                "Failed to parse RegisterReimbursementKickoffInput: {e}"
            ))
        })?;

        let tx_hash = self
            .contract
            .invoke_register_reimbursement_kickoff(
                input.accept_pegin_txid,
                parsed_input,
                self.gas_bumps,
            )
            .await?;

        info!("RegisterReimbursementKickoff successful at tx {tx_hash}");
        Ok(RegisterReimbursementKickoffOutput { transaction_hash: tx_hash.to_string() })
    }
}
