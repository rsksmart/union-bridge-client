use std::str::FromStr;
use alloy_primitives::FixedBytes;
use anyhow::Result;
use common::msg_broker::bitvmx_types::PubKeyHash;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    ECDSAPublicKey, MemberRegistrationKeys, RSAPublicKey,
};
use crate::rsk_gateway::DomainErrors;
use crate::types::{CommitteeECDSA, P2PAddressParser};
pub type FixedBytes32 = FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type Address = alloy_primitives::Address;

pub fn convert_to_member_registration_keys(
    take_key_data: &CommitteeECDSA,
    dispute_key_data: &CommitteeECDSA,
    pubkey_hash: &PubKeyHash,
) -> Result<MemberRegistrationKeys, DomainErrors> {
    let take_key = ECDSAPublicKey {
        publicKeyX: crate::contracts::types::FixedBytes32::from_str(&take_key_data.x)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key x value".to_string()))?,
        publicKeyY: crate::contracts::types::FixedBytes32::from_str(&take_key_data.y)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key y value".to_string()))?,
        r: crate::contracts::types::FixedBytes32::from_str(&take_key_data.r)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key r value".to_string()))?,
        s: crate::contracts::types::FixedBytes32::from_str(&take_key_data.s)
            .map_err(|_| DomainErrors::InvalidPublicKey("Invalid take key s value".to_string()))?,
        v: match take_key_data.v {
            27 | 28 => take_key_data.v,
            _ => {
                return Err(DomainErrors::InvalidValue("Invalid take key v value".to_string()));
            }
        },
    };

    let covenant_key = ECDSAPublicKey {
        publicKeyX: crate::contracts::types::FixedBytes32::from_str(&dispute_key_data.x).map_err(
            |_| DomainErrors::InvalidPublicKey("Invalid covenant key x value".to_string()),
        )?,
        publicKeyY: crate::contracts::types::FixedBytes32::from_str(&dispute_key_data.y).map_err(
            |_| DomainErrors::InvalidPublicKey("Invalid covenant key y value".to_string()),
        )?,
        r: crate::contracts::types::FixedBytes32::from_str(&dispute_key_data.r).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key r value".to_string())
        })?,
        s: crate::contracts::types::FixedBytes32::from_str(&dispute_key_data.s).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key s value".to_string())
        })?,
        v: match dispute_key_data.v {
            27 | 28 => dispute_key_data.v,
            _ => {
                return Err(DomainErrors::InvalidValue("Invalid covenant key v value".to_string()));
            }
        },
    };

    let pubkey_hash_as_rsa = P2PAddressParser::pubkey_hash_to_contracts(pubkey_hash).map_err(|_| {
        DomainErrors::InvalidPublicKey(format!("Cannot parse pubkey_hash: {pubkey_hash}"))
    })?;

    Ok(MemberRegistrationKeys {
        takeKey: take_key,
        covenantKey: covenant_key,
        communicationKey: RSAPublicKey {
            rsaPublicKey: pubkey_hash_as_rsa.rsaPublicKey,
        },
    })
}
