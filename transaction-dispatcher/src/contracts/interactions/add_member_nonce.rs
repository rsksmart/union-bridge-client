use crate::contracts;
use crate::contracts::signature_manager::SignatureManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{AddMemberNonceInput, AddMemberNonceOutput};
use log::{error, info};

#[derive(Clone)]
pub(crate) struct AddMemberNonceInvoke<C: SignatureManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: SignatureManagerContractApi> AddMemberNonceInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        AddMemberNonceInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: AddMemberNonceInput,
    ) -> Result<AddMemberNonceOutput, DomainErrors> {
        info!("Init AddMemberNonce for: {input:?}");

        let hash_to_sign = input.hash_to_sign.into();
        let nonce = contracts::types::Bytes::from(input.nonce.serialize());

        let receipt = self
            .contract
            .add_member_nonce(hash_to_sign, nonce, self.gas_bumps)
            .await?;

        if receipt.status() {
            info!(
                "AddMemberNonce successful at tx {}",
                receipt.transaction_hash
            );
            Ok(AddMemberNonceOutput {
                transaction_hash: receipt.transaction_hash.to_string(),
            })
        } else {
            error!("AddMemberNonce failed at tx {}", receipt.transaction_hash);
            Err(DomainErrors::TransactionFailed(format!(
                "AddMemberNonce transaction failed with receipt status false at tx {}",
                receipt.transaction_hash
            )))
        }
    }
}
