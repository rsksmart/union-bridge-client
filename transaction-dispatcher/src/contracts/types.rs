use crate::types::CommitteePublicKey;
use alloy_primitives::FixedBytes;
use alloy_rpc_types::TransactionReceipt;
use std::str::FromStr;
use union_contracts::bindings::committee_registry::CommitteeRegistry::PublicKeyRegistration;

pub type FixedBytes32 = FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type TransactionReceiptResult = alloy_contract::Result<TransactionReceipt>;
pub type Address = alloy_primitives::Address;

impl TryFrom<CommitteePublicKey> for PublicKeyRegistration {
    type Error = alloy_primitives::hex::FromHexError;

    fn try_from(value: CommitteePublicKey) -> Result<Self, Self::Error> {
        Ok(PublicKeyRegistration {
            publicKeyX: FixedBytes32::from_str(&value.x)?,
            publicKeyY: FixedBytes32::from_str(&value.y)?,
            r: FixedBytes32::from_str(&value.r)?,
            s: FixedBytes32::from_str(&value.s)?,
            v: value.v.into(),
        })
    }
}
