use crate::contracts::signature_manager::SignatureManagerContractApi;
use crate::contracts::types::FixedBytes32;
use crate::rsk_gateway::DomainErrors;
use crate::types::{AddOperatorTakeTxHashInput, AddOperatorTakeTxHashOutput};
use bitcoin::hashes::Hash;
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
        info!("Init AddOperatorTakeTxHash for: {:?}", input);

        let accept_pegin_tx_hash =
            FixedBytes32::from_slice(input.accept_pegin_tx_hash.as_raw_hash().as_byte_array());
        let take_tx_hash = FixedBytes32::from_slice(input.take_tx_hash.as_slice());

        let receipt = self
            .contract
            .add_operator_take_tx_hash(accept_pegin_tx_hash, take_tx_hash, self.gas_bumps)
            .await?;

        let result = match receipt.status() {
            true => {
                info!(
                    "AddOperatorTakeTxHash successful at tx {}",
                    receipt.transaction_hash
                );
                AddOperatorTakeTxHashOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: true,
                }
            }
            false => {
                error!(
                    "AddMemberSignature failed at tx {}",
                    receipt.transaction_hash
                );
                AddOperatorTakeTxHashOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: false,
                }
            }
        };

        Ok(result)
    }
}
