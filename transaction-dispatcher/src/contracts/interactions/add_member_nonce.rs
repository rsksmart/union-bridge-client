use crate::contracts::signature_manager::{
    SignatureManagerContractApi, hex_to_bytes, hex_to_fixed_bytes32,
};
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
        info!("Init AddMemberNonce for: {:?}", input);

        let hash_to_sign = hex_to_fixed_bytes32(&input.hash_to_sign)
            .map_err(|e| DomainErrors::InvalidValue(format!("Invalid hash_to_sign: {}", e)))?;

        let nonce = hex_to_bytes(&input.nonce)
            .map_err(|e| DomainErrors::InvalidValue(format!("Invalid nonce hex: {}", e)))?;

        let receipt = self
            .contract
            .add_member_nonce(hash_to_sign, nonce, self.gas_bumps)
            .await?;

        let result = match receipt.status() {
            true => {
                info!(
                    "AddMemberNonce successful at tx {}",
                    receipt.transaction_hash
                );
                AddMemberNonceOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: true,
                }
            }
            false => {
                error!("AddMemberNonce failed at tx {}", receipt.transaction_hash);
                AddMemberNonceOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: false,
                }
            }
        };

        Ok(result)
    }
}
