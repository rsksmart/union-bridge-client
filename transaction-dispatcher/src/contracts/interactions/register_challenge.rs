use log::info;
use union_contracts::bindings::challenge_manager::ChallengeManager::BtcTxSPVProof;

use crate::contracts::challenge_manager::ChallengeManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterChallengeInput, RegisterChallengeOutput};

#[derive(Clone)]
pub(crate) struct RegisterChallengeInvoke<C: ChallengeManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: ChallengeManagerContractApi> RegisterChallengeInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterChallengeInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterChallengeInput,
    ) -> Result<RegisterChallengeOutput, DomainErrors> {
        info!("Init RegisterChallenge for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.challenge_spv_proof.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!(
                "Failed to parse RegisterChallengeInput: {e}"
            ))
        })?;

        let tx_hash = self
            .contract
            .invoke_register_challenge(input.accept_pegin_txid, parsed_input, self.gas_bumps)
            .await?;

        info!("RegisterChallenge successful at tx {tx_hash}");
        Ok(RegisterChallengeOutput { transaction_hash: tx_hash.to_string() })
    }
}
