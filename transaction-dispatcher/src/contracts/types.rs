use crate::types::CommitteePublicKey;
use alloy_primitives::FixedBytes;
use alloy_rpc_types::TransactionReceipt;
use union_contracts::bindings::committee_registry::CommitteeRegistry::PublicKeyRegistration;

pub type FixedBytes32 = FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type TransactionReceiptResult = alloy_contract::Result<TransactionReceipt>;
pub type Address = alloy_primitives::Address;

impl From<CommitteePublicKey> for PublicKeyRegistration {
    fn from(key: CommitteePublicKey) -> Self {
        PublicKeyRegistration {
            publicKeyX: FixedBytes::from(key.x),
            publicKeyY: FixedBytes::from(key.y),
            r: FixedBytes::from(key.r),
            s: FixedBytes::from(key.s),
            v: key.v,
        }
    }
}
