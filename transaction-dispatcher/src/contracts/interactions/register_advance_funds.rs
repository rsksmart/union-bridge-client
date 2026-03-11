use log::info;
use union_contracts::bindings::pegout_manager::PegoutManager::BtcTxSPVProof;

use crate::contracts::pegout_manager::PegoutManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterAdvanceFundsInput, RegisterAdvanceFundsOutput};

#[derive(Clone)]
pub(crate) struct RegisterAdvanceFundsInvoke<C: PegoutManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegoutManagerContractApi> RegisterAdvanceFundsInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterAdvanceFundsInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterAdvanceFundsInput,
    ) -> Result<RegisterAdvanceFundsOutput, DomainErrors> {
        info!("Init RegisterAdvanceFunds for: {input:?}");

        let parsed_input: BtcTxSPVProof =
            input.advance_funds_spv_proof.try_into().map_err(|e| {
                DomainErrors::InvalidBtcTxSpvProof(format!(
                    "Failed to parse RegisterAdvanceFundsInput: {e}"
                ))
            })?;

        let tx_hash = self
            .contract
            .invoke_register_advance_funds(input.accept_pegin_txid, parsed_input, self.gas_bumps)
            .await?;

        info!("RegisterAdvanceFunds successful at tx {tx_hash}");
        Ok(RegisterAdvanceFundsOutput { transaction_hash: tx_hash.to_string() })
    }
}
