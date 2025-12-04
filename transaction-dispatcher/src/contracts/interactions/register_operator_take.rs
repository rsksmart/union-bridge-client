use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterOperatorTakeInput, RegisterOperatorTakeOutput};
use log::info;
use union_contracts::bindings::peg_manager::PegManager::BtcTxSPVProof;

#[derive(Clone)]
pub(crate) struct RegisterOperatorTakeInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> RegisterOperatorTakeInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterOperatorTakeInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterOperatorTakeInput,
    ) -> Result<RegisterOperatorTakeOutput, DomainErrors> {
        info!("Init RegisterOperatorTake for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!(
                "Failed to parse RegisterOperatorTakeInput: {e}"
            ))
        })?;

        let transaction_hash =
            self.contract.invoke_register_operator_take(parsed_input, self.gas_bumps).await?;

        Ok(RegisterOperatorTakeOutput { transaction_hash: transaction_hash.to_string() })
    }
}

//todo tests
