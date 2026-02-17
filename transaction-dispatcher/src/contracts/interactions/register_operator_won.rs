use log::info;
use union_contracts::bindings::pegout_manager::PegoutManager::BtcTxSPVProof;

use crate::contracts::pegout_manager::PegoutManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterOperatorWonInput, RegisterOperatorWonOutput};

#[derive(Clone)]
pub(crate) struct RegisterOperatorWonInvoke<C: PegoutManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegoutManagerContractApi> RegisterOperatorWonInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterOperatorWonInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterOperatorWonInput,
    ) -> Result<RegisterOperatorWonOutput, DomainErrors> {
        info!("Init RegisterOperatorWon for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!(
                "Failed to parse RegisterOperatorWonInput: {e}"
            ))
        })?;

        let tx_hash =
            self.contract.invoke_register_operator_won(parsed_input, self.gas_bumps).await?;

        info!("RegisterOperatorWon successful at tx {tx_hash}");
        Ok(RegisterOperatorWonOutput { transaction_hash: tx_hash.to_string() })
    }
}
