use crate::contracts::signature_manager::{SignatureManagerContractApi, hex_to_fixed_bytes32};
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

        let hash_to_sign = hex_to_fixed_bytes32(&input.hash_to_sign)
            .map_err(|e| DomainErrors::InvalidValue(format!("Invalid hash_to_sign: {}", e)))?;

        let signature = hex_to_fixed_bytes32(&input.signature)
            .map_err(|e| DomainErrors::InvalidValue(format!("Invalid signature: {}", e)))?;

        let receipt = self
            .contract
            .add_member_signature(hash_to_sign, signature, self.gas_bumps)
            .await?;

        let result = match receipt.status() {
            true => {
                info!(
                    "AddMemberSignature successful at tx {}",
                    receipt.transaction_hash
                );
                AddMemberSignatureOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: true,
                }
            }
            false => {
                error!(
                    "AddMemberSignature failed at tx {}",
                    receipt.transaction_hash
                );
                AddMemberSignatureOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: false,
                }
            }
        };

        Ok(result)
    }
}
