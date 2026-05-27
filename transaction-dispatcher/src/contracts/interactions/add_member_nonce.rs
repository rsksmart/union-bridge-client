use tracing::info;

use crate::contracts;
use crate::contracts::signature_manager::SignatureManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{AddMemberNonceInput, AddMemberNonceOutput};

#[derive(Clone)]
pub(crate) struct AddMemberNonceInvoke<C: SignatureManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: SignatureManagerContractApi> AddMemberNonceInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        AddMemberNonceInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: AddMemberNonceInput,
    ) -> Result<AddMemberNonceOutput, DomainErrors> {
        info!("Init AddMemberNonce for: {input:?}");

        let hash_to_sign = input.hash_to_sign.into();
        let nonce = contracts::types::Bytes::from(input.nonce.serialize());

        let tx_hash = self.contract.add_member_nonce(hash_to_sign, nonce, self.gas_bumps).await?;

        info!("AddMemberNonce successful at tx {tx_hash}");
        Ok(AddMemberNonceOutput { transaction_hash: tx_hash.to_string() })
    }
}
