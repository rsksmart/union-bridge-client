use crate::rsk_gateway::DomainErrors;
use crate::types::{CommitteeECDSA, CommitteeRSA};
use alloy_primitives::FixedBytes;
use alloy_rpc_types::TransactionReceipt;
use anyhow::Result;
use std::str::FromStr;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    ECDSAPublicKey, MemberRegistrationKeys, RSAPublicKey,
};

pub type FixedBytes32 = FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type TransactionReceiptResult = alloy_contract::Result<TransactionReceipt>;
pub type Address = alloy_primitives::Address;

pub fn convert_to_member_registration_keys(
    take_key_data: &CommitteeECDSA,
    dispute_key_data: &CommitteeECDSA,
    communication_key_data: &CommitteeRSA,
) -> Result<MemberRegistrationKeys, DomainErrors> {
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

    let covenant_key = ECDSAPublicKey {
        publicKeyX: FixedBytes32::from_str(&dispute_key_data.x).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key x value".to_string())
        })?,
        publicKeyY: FixedBytes32::from_str(&dispute_key_data.y).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key y value".to_string())
        })?,
        r: FixedBytes32::from_str(&dispute_key_data.r).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key r value".to_string())
        })?,
        s: FixedBytes32::from_str(&dispute_key_data.s).map_err(|_| {
            DomainErrors::InvalidPublicKey("Invalid covenant key s value".to_string())
        })?,
        v: match dispute_key_data.v {
            27 | 28 => dispute_key_data.v,
            _ => {
                return Err(DomainErrors::InvalidValue(
                    "Invalid covenant key v value".to_string(),
                ));
            }
        },
    };

    let communication_key = hex_to_rsa(communication_key_data).map_err(|_| {
        DomainErrors::InvalidPublicKey(format!(
            "Cannot parse communication data {communication_key_data}"
        ))
    })?;

    Ok(MemberRegistrationKeys {
        takeKey: take_key,
        covenantKey: covenant_key,
        communicationKey: RSAPublicKey {
            rsaPublicKey: communication_key,
        },
    })
}

type RsaKeyContracts = [FixedBytes<32>; 10];

/// Encode `[FixedBytes<32>; 10]` into a 0x-hex string.
pub fn rsa_to_hex(arr: &RsaKeyContracts) -> String {
    let mut buf = [0u8; 10 * 32];
    for (i, fb) in arr.iter().enumerate() {
        buf[i * 32..(i + 1) * 32].copy_from_slice(&fb.0);
    }

    let mut out = String::with_capacity(2 + buf.len() * 2);
    out.push_str("0x");
    out.push_str(&hex::encode(buf));
    out
}

/// Decode hex into `[FixedBytes<32>; 10]`.
/// - If input < 320 bytes → zero-pad to 320.
/// - If input > 320 bytes → return `InvalidStringLength`.
pub fn hex_to_rsa(s: &str) -> Result<RsaKeyContracts, hex::FromHexError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let mut bytes = hex::decode(s)?;

    const N: usize = 10 * 32;
    if bytes.len() > N {
        return Err(hex::FromHexError::InvalidStringLength);
    }

    // Pad with zeros if shorter
    bytes.resize(N, 0);

    Ok(std::array::from_fn(|i| {
        let mut chunk = [0u8; 32];
        chunk.copy_from_slice(&bytes[i * 32..(i + 1) * 32]);
        FixedBytes::<32>::from(chunk)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let input: RsaKeyContracts = std::array::from_fn(|i| {
            let mut b = [0u8; 32];
            for (j, x) in b.iter_mut().enumerate() {
                *x = (i as u8) ^ (j as u8);
            }
            FixedBytes::<32>::from(b)
        });

        let s = rsa_to_hex(&input);
        let out = hex_to_rsa(&s).unwrap();
        assert_eq!(input, out);
    }

    #[test]
    fn invalid_rsa_hex() {
        let invalid_hex = "0x0298d519293a38236d7f0355d5b50d941c18dc5488b7bdc950f06625f9d5685c6387330da5d92a6e6d02d7996abe087ea3bd8c3488379b9f9e00dd21a0581e1fb11945e8e46bad3559e7463bf681903183d60b44676f2466c8d439ebb48f9bfd7400";
        let result = hex_to_rsa(invalid_hex);
        assert!(result.is_ok(), "Expected valid hex to decode successfully");
    }
}
