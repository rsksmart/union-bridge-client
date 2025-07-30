use crate::rsk_gateway::DomainErrors;
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
    type Error = DomainErrors;

    fn try_from(value: CommitteePublicKey) -> Result<Self, Self::Error> {
        Ok(PublicKeyRegistration {
            publicKeyX: FixedBytes32::from_str(&value.x)
                .map_err(|_| DomainErrors::InvalidPublicKey("Invalid x value".to_string()))?,
            publicKeyY: FixedBytes32::from_str(&value.y)
                .map_err(|_| DomainErrors::InvalidPublicKey("Invalid y value".to_string()))?,
            r: FixedBytes32::from_str(&value.r)
                .map_err(|_| DomainErrors::InvalidPublicKey("Invalid r value".to_string()))?,
            s: FixedBytes32::from_str(&value.s)
                .map_err(|_| DomainErrors::InvalidPublicKey("Invalid s value".to_string()))?,
            v: match value.v {
                27 | 28 => value.v,
                _ => return Err(DomainErrors::InvalidValue("Invalid v value".to_string())),
            },
        })
    }
}
