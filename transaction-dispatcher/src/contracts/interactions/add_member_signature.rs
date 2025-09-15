use crate::contracts::signature_manager::SignatureManagerContractApi;
use crate::contracts::types::FixedBytes32;
use crate::rsk_gateway::DomainErrors;
use crate::types::{AddMemberSignatureInput, AddMemberSignatureOutput};
use log::{error, info};

#[derive(Clone)]
pub(crate) struct AddMemberSignatureInvoke<C: SignatureManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: SignatureManagerContractApi> AddMemberSignatureInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        AddMemberSignatureInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: AddMemberSignatureInput,
    ) -> Result<AddMemberSignatureOutput, DomainErrors> {
        info!("Init AddMemberSignature for: {:?}", input);

        let hash_to_sign = input.hash_to_sign.into();
        let signature = FixedBytes32::from_slice(&input.signature.serialize());

        let receipt = self
            .contract
            .add_member_signature(hash_to_sign, signature, self.gas_bumps)
            .await?;

        match receipt.status() {
            true => {
                info!(
                    "AddMemberSignature successful at tx {}",
                    receipt.transaction_hash
                );
                Ok(AddMemberSignatureOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                })
            }
            false => {
                error!(
                    "AddMemberSignature failed at tx {}",
                    receipt.transaction_hash
                );
                Err(DomainErrors::TransactionFailed(format!(
                    "AddMemberSignature transaction failed with receipt status false at tx {}",
                    receipt.transaction_hash
                )))
            }
        }
    }
}
