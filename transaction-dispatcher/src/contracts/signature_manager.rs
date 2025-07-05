use crate::contracts::types::{Address, Bytes, FixedBytes32, TransactionReceiptResult};
use alloy_provider::Provider;
use log::info;

use crate::contracts::common::send_tx_with_gas_bump;
#[cfg(test)]
use mockall::automock;
use union_contracts::bindings::signature_manager::SignatureManager;
use union_contracts::bindings::signature_manager::SignatureManager::SignatureManagerInstance;

#[cfg_attr(test, automock)]
pub trait SignatureManagerContractApi {
    async fn add_member_nonce(
        &self,
        hash_to_sign: FixedBytes32,
        nonce: Bytes,
        gas_bumps: u8,
    ) -> TransactionReceiptResult;

    async fn add_member_signature(
        &self,
        hash_to_sign: FixedBytes32,
        signature: FixedBytes32,
        gas_bumps: u8,
    ) -> TransactionReceiptResult;
}

#[derive(Clone)]
pub struct SignatureManagerContract<P: Provider> {
    contract_instance: SignatureManagerInstance<P>,
}

impl<P: Provider> SignatureManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!(
            "Connecting to SignatureManager Contract @ {}",
            contract_address
        );
        let contract_instance = SignatureManager::new(contract_address, provider);
        SignatureManagerContract { contract_instance }
    }
}

impl<P: Provider> SignatureManagerContractApi for SignatureManagerContract<P> {
    async fn add_member_nonce(
        &self,
        hash_to_sign: FixedBytes32,
        nonce: Bytes,
        gas_bumps: u8,
    ) -> TransactionReceiptResult {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .addMemberNonce(hash_to_sign.clone(), nonce.clone())
            },
            gas_bumps,
        )
        .await
    }

    async fn add_member_signature(
        &self,
        hash_to_sign: FixedBytes32,
        signature: FixedBytes32,
        gas_bumps: u8,
    ) -> TransactionReceiptResult {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .addMemberSignature(hash_to_sign.clone(), signature.clone())
            },
            gas_bumps,
        )
        .await
    }
}
