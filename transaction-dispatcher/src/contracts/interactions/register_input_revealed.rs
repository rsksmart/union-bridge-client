use log::info;
use union_contracts::bindings::challenge_manager::ChallengeManager::BtcTxSPVProof;

use crate::contracts::challenge_manager::ChallengeManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterInputRevealedInput, RegisterInputRevealedOutput};

#[derive(Clone)]
pub(crate) struct RegisterInputRevealedInvoke<C: ChallengeManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: ChallengeManagerContractApi> RegisterInputRevealedInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterInputRevealedInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterInputRevealedInput,
    ) -> Result<RegisterInputRevealedOutput, DomainErrors> {
        info!("Init RegisterInputRevealed for: {input:?}");

        let parsed_input: BtcTxSPVProof =
            input.input_revealed_spv_proof.try_into().map_err(|e| {
                DomainErrors::InvalidBtcTxSpvProof(format!(
                    "Failed to parse RegisterInputRevealedInput: {e}"
                ))
            })?;

        let tx_hash = self
            .contract
            .invoke_register_input_revealed(input.accept_pegin_txid, parsed_input, self.gas_bumps)
            .await?;

        info!("RegisterInputRevealed successful at tx {tx_hash}");
        Ok(RegisterInputRevealedOutput { transaction_hash: tx_hash.to_string() })
    }
}
