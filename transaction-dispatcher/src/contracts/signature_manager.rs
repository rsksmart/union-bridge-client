use crate::contracts::types::{Address, Bytes, FixedBytes32, TransactionReceiptResult};
use alloy_provider::Provider;
use hex::FromHexError;
use log::info;

use crate::contracts::common::send_tx_with_gas_bump;
#[cfg(test)]
use mockall::automock;
use union_contracts::bindings::signature_manager::SignatureManager;
use union_contracts::bindings::signature_manager::SignatureManager::SignatureManagerInstance;

pub(crate) use crate::contracts::interactions::add_member_nonce::AddMemberNonceInvoke;
pub(crate) use crate::contracts::interactions::add_member_signature::AddMemberSignatureInvoke;

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

pub fn hex_to_fixed_bytes32(value: &str) -> Result<FixedBytes32, FromHexError> {
    let value = value.trim_start_matches("0x");
    let bytes = hex::decode(value)?;
    Ok(FixedBytes32::from_slice(&bytes))
}

pub fn hex_to_bytes(value: &str) -> Result<Bytes, FromHexError> {
    let value = value.trim_start_matches("0x");
    let bytes = hex::decode(value)?;
    Ok(Bytes::from(bytes))
}

pub(crate) fn decode_error(
    err: &alloy_contract::Error,
) -> Option<crate::rsk_gateway::DomainErrors> {
    use crate::rsk_gateway::DomainErrors;
    use union_contracts::bindings::signature_manager::SignatureManager::SignatureManagerErrors;

    let decoded_err = err.as_decoded_interface_error::<SignatureManagerErrors>();
    decoded_err.map(|e| match e {
        SignatureManagerErrors::AcceptPeginTxHashNotFound(e) => {
            DomainErrors::InvalidValue(format!("AcceptPeginTxHashNotFound: {:?}", e))
        }
        SignatureManagerErrors::AddressEmptyCode(e) => {
            DomainErrors::InvalidAddress(format!("AddressEmptyCode: {:?}", e))
        }
        SignatureManagerErrors::HashToSignNotFound(e) => {
            DomainErrors::InvalidValue(format!("HashToSignNotFound: {:?}", e))
        }
        // TODO handle more based on needs
        _ => DomainErrors::UnhandledContractError(format!("{:?}", e)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;
    use alloy_primitives::{Address, FixedBytes};
    use union_contracts::bindings::signature_manager::SignatureManager::{
        AcceptPeginTxHashNotFound, AddressEmptyCode, HashToSignNotFound, SignatureManagerErrors,
    };

    // Test helper functions
    #[test]
    fn test_hex_to_fixed_bytes32_valid() {
        let hex_str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = hex_to_fixed_bytes32(hex_str);
        assert!(result.is_ok());

        let expected_bytes = [
            0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
            0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78,
            0x90, 0xab, 0xcd, 0xef,
        ];
        let fixed_bytes = result.unwrap();
        assert_eq!(fixed_bytes.0, expected_bytes);
    }

    #[test]
    fn test_hex_to_fixed_bytes32_without_prefix() {
        let hex_str = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = hex_to_fixed_bytes32(hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_hex_to_fixed_bytes32_invalid_chars() {
        let hex_str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdeg"; // 'g' is invalid
        let result = hex_to_fixed_bytes32(hex_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_to_bytes_valid() {
        let hex_str = "0x1234abcd";
        let result = hex_to_bytes(hex_str);
        assert!(result.is_ok());

        let bytes = result.unwrap();
        assert_eq!(bytes.as_ref(), &[0x12, 0x34, 0xab, 0xcd]);
    }

    #[test]
    fn test_hex_to_bytes_without_prefix() {
        let hex_str = "1234abcd";
        let result = hex_to_bytes(hex_str);
        assert!(result.is_ok());

        let bytes = result.unwrap();
        assert_eq!(bytes.as_ref(), &[0x12, 0x34, 0xab, 0xcd]);
    }

    #[test]
    fn test_hex_to_bytes_empty() {
        let hex_str = "0x";
        let result = hex_to_bytes(hex_str);
        assert!(result.is_ok());

        let bytes = result.unwrap();
        assert_eq!(bytes.as_ref(), &[] as &[u8]);
    }

    #[test]
    fn test_hex_to_bytes_invalid_chars() {
        let hex_str = "0x1234gg";
        let result = hex_to_bytes(hex_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_to_bytes_odd_length() {
        let hex_str = "0x123"; // Odd length should fail
        let result = hex_to_bytes(hex_str);
        assert!(result.is_err());
    }

    // Test error decoding functions
    #[test]
    fn test_accept_pegin_tx_hash_not_found_error() {
        let err_data =
            SignatureManagerErrors::AcceptPeginTxHashNotFound(AcceptPeginTxHashNotFound {
                acceptPeginTxHash: FixedBytes::<32>::from([1u8; 32]),
            });

        let result = generate_contract_revert_error(err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::InvalidValue(_)));
    }

    #[test]
    fn test_address_empty_code_error() {
        let err_data = SignatureManagerErrors::AddressEmptyCode(AddressEmptyCode {
            target: Address::default(),
        });

        let result = generate_contract_revert_error(err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::InvalidAddress(_)));
    }

    #[test]
    fn test_hash_to_sign_not_found_error() {
        let err_data = SignatureManagerErrors::HashToSignNotFound(HashToSignNotFound {
            hashToSign: FixedBytes::<32>::from([7u8; 32]),
        });

        let result = generate_contract_revert_error(err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::InvalidValue(_)));
    }

    // Test unhandled error mapping
    #[test]
    fn test_unhandled_error() {
        use union_contracts::bindings::signature_manager::SignatureManager::ERC1967InvalidImplementation;

        let err_data =
            SignatureManagerErrors::ERC1967InvalidImplementation(ERC1967InvalidImplementation {
                implementation: Address::default(),
            });

        let result = generate_contract_revert_error(err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(
            domain_error,
            DomainErrors::UnhandledContractError(_)
        ));
    }
}
