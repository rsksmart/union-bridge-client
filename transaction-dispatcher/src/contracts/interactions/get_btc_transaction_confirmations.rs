use crate::contracts::native_bridge::NativeBridgeContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{GetBtcTransactionConfirmationsInput, GetBtcTransactionConfirmationsOutput};

#[derive(Clone)]
pub struct GetBtcTransactionConfirmationsCall<C: NativeBridgeContractApi> {
    contract: C,
}

impl<C: NativeBridgeContractApi> GetBtcTransactionConfirmationsCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        Self { contract }
    }

    pub(crate) async fn run(
        &self,
        input: GetBtcTransactionConfirmationsInput,
    ) -> Result<GetBtcTransactionConfirmationsOutput, DomainErrors> {
        let confirmations = self
            .contract
            .call_get_btc_transaction_confirmations(
                input.tx_hash,
                input.block_hash,
                input.merkle_branch_path,
                input.merkle_branch_hashes,
            )
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get bitcoin confirmations: {e}"
                ))
            })?;

        Ok(GetBtcTransactionConfirmationsOutput { confirmations })
    }
}
