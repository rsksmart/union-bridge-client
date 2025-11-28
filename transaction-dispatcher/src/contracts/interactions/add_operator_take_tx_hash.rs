use crate::contracts::signature_manager::SignatureManagerContractApi;
use crate::contracts::types::FixedBytes32;
use crate::rsk_gateway::DomainErrors;
use crate::types::{AddOperatorTakeTxHashInput, AddOperatorTakeTxHashOutput};
use common::types::TxIdParser;
use log::{error, info};

#[derive(Clone)]
pub(crate) struct AddOperatorTakeTxHashInvoke<C: SignatureManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: SignatureManagerContractApi> AddOperatorTakeTxHashInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        Self {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: AddOperatorTakeTxHashInput,
    ) -> Result<AddOperatorTakeTxHashOutput, DomainErrors> {
        info!("Init AddOperatorTakeTxHash for: {input:?}");

        let accept_pegin_tx_hash = TxIdParser::txid_to_fb_32(input.accept_pegin_tx_hash);
        let take_tx_hash = FixedBytes32::from_slice(input.take_tx_hash.as_slice());

        let receipt = self
            .contract
            .add_operator_take_tx_hash(accept_pegin_tx_hash, take_tx_hash, self.gas_bumps)
            .await?;

        if receipt.status() {
            info!(
                "AddOperatorTakeTxHash successful at tx {}",
                receipt.transaction_hash
            );
            Ok(AddOperatorTakeTxHashOutput {
                transaction_hash: receipt.transaction_hash.to_string(),
            })
        } else {
            error!(
                "AddOperatorTakeTxHash failed at tx {}",
                receipt.transaction_hash
            );
            Err(DomainErrors::TransactionFailed(format!(
                "AddOperatorTakeTxHash transaction failed with receipt status false at tx {}",
                receipt.transaction_hash
            )))
        }
    }
}
