use crate::rsk_gateway::DomainErrors;
use crate::types::CommitteePublicKey;
use alloy_primitives::FixedBytes;
use alloy_rpc_types::TransactionReceipt;
use std::str::FromStr;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{MemberRegistrationKeys, ECDSAPublicKey, RSAPublicKey};

pub type FixedBytes32 = FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type TransactionReceiptResult = alloy_contract::Result<TransactionReceipt>;
pub type Address = alloy_primitives::Address;

pub fn convert_to_member_registration_keys(value: [CommitteePublicKey; 3]) -> Result<MemberRegistrationKeys, DomainErrors> {
    let take_key = ECDSAPublicKey {
        publicKeyX: FixedBytes32::from_str(&value[0].x)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key x value".to_string()))?,
        publicKeyY: FixedBytes32::from_str(&value[0].y)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key y value".to_string()))?,
        r: FixedBytes32::from_str(&value[0].r)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key r value".to_string()))?,
        s: FixedBytes32::from_str(&value[0].s)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key s value".to_string()))?,
        v: match value[0].v {
            27 | 28 => value[0].v,
            _ => return Err(DomainErrors::InvalidValue("Invalid take key v value".to_string())),
        },
    };
    
    let covenant_key = ECDSAPublicKey {
        publicKeyX: FixedBytes32::from_str(&value[1].x)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid covenant key x value".to_string()))?,
        publicKeyY: FixedBytes32::from_str(&value[1].y)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid covenant key y value".to_string()))?,
        r: FixedBytes32::from_str(&value[1].r)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid covenant key r value".to_string()))?,
        s: FixedBytes32::from_str(&value[1].s)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid covenant key s value".to_string()))?,
        v: match value[1].v {
            27 | 28 => value[1].v,
            _ => return Err(DomainErrors::InvalidValue("Invalid covenant key v value".to_string())),
        },
    };
    
    // For communication key, we use the third CommitteePublicKey's x value
    // to populate the first element of the RSA key array
    // This maintains backward compatibility with the existing API
    let mut rsa_array = [FixedBytes32::default(); 10];
    rsa_array[0] = FixedBytes32::from_str(&value[2].x)
        .map_err(|_| DomainErrors::InvalidPublicKey("Invalid communication key value".to_string()))?;
    
    Ok(MemberRegistrationKeys {
        takeKey: take_key,
        covenantKey: covenant_key,
        communicationKey: RSAPublicKey {
            rsaPublicKey: rsa_array,
        },
    })
}
