use crate::rsk_gateway::DomainErrors;
use crate::types::CommitteePublicKey;
use alloy_primitives::FixedBytes;
use alloy_rpc_types::TransactionReceipt;
use std::str::FromStr;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    ECDSAPublicKey, MemberRegistrationKeys, RSAPublicKey,
};

pub type FixedBytes32 = FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type TransactionReceiptResult = alloy_contract::Result<TransactionReceipt>;
pub type Address = alloy_primitives::Address;

pub fn convert_to_member_registration_keys(
    value: Vec<CommitteePublicKey>,
) -> Result<MemberRegistrationKeys, DomainErrors> {
    let take_key_data = value
        .get(0)
        .ok_or_else(|| DomainErrors::InvalidPublicKey("Missing take key".to_string()))?;

    let take_key = ECDSAPublicKey {
        publicKeyX: FixedBytes32::from_str(&take_key_data.x)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key x value".to_string()))?,
        publicKeyY: FixedBytes32::from_str(&take_key_data.y)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key y value".to_string()))?,
        r: FixedBytes32::from_str(&take_key_data.r)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key r value".to_string()))?,
        s: FixedBytes32::from_str(&take_key_data.s)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key s value".to_string()))?,
        v: match take_key_data.v {
            27 | 28 => take_key_data.v,
            _ => {
                return Err(DomainErrors::InvalidValue(
                    "Invalid take key v value".to_string(),
                ));
            }
        },
    };

    let covenant_key_data = value
        .get(1)
        .ok_or_else(|| DomainErrors::InvalidPublicKey("Missing covenant key".to_string()))?;

    let covenant_key = ECDSAPublicKey {
        publicKeyX: FixedBytes32::from_str(&covenant_key_data.x).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key x value".to_string())
        })?,
        publicKeyY: FixedBytes32::from_str(&covenant_key_data.y).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key y value".to_string())
        })?,
        r: FixedBytes32::from_str(&covenant_key_data.r).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key r value".to_string())
        })?,
        s: FixedBytes32::from_str(&covenant_key_data.s).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key s value".to_string())
        })?,
        v: match covenant_key_data.v {
            27 | 28 => covenant_key_data.v,
            _ => {
                return Err(DomainErrors::InvalidValue(
                    "Invalid covenant key v value".to_string(),
                ));
            }
        },
    };

    // TODO(iago) address this
    // For communication key, we use the third CommitteePublicKey's x value
    // to populate the first element of the RSA key array
    // This maintains backward compatibility with the existing API
    let communication_key_data = value
        .get(2)
        .ok_or_else(|| DomainErrors::InvalidPublicKey("Missing communication key".to_string()))?;

    let mut rsa_array = [FixedBytes32::default(); 10];
    *rsa_array
        .get_mut(0)
        .ok_or_else(|| DomainErrors::InvalidPublicKey("Invalid RSA array access".to_string()))? =
        FixedBytes32::from_str(&communication_key_data.x).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid communication key value".to_string())
        })?;

    Ok(MemberRegistrationKeys {
        takeKey: take_key,
        covenantKey: covenant_key,
        communicationKey: RSAPublicKey {
            rsaPublicKey: rsa_array,
        },
    })
}
