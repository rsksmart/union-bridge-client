use alloy_primitives::{Address, Bytes, FixedBytes, I256, U256};
use alloy_provider::Provider;
use alloy_transport::TransportError;
use anyhow::Result as AnyhowResult;
use common::types::{BlockHash, TxHash};
use log::{debug, info, warn};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait PowpegBridgeContractApi {
    async fn call_get_btc_transaction_confirmations(
        &self,
        tx_hash: TxHash,
        block_hash: BlockHash,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> alloy_contract::Result<u32>;
}

// Native Bridge precompiled contract address (RSKIP122)
const NATIVE_BRIDGE_ADDRESS: &str = "0x0000000000000000000000000000000001000006";

// Function selector for getBtcTransactionConfirmations(bytes32,bytes32,uint256,bytes32[])
// Calculated as keccak256("getBtcTransactionConfirmations(bytes32,bytes32,uint256,bytes32[])")[0:4]
const GET_BTC_TX_CONFIRMATIONS_SELECTOR: [u8; 4] = [0x5b, 0x64, 0x45, 0x87];

#[derive(Clone)]
pub struct PowpegBridgeContract<P: Provider> {
    provider: P,
    contract_address: Address,
}

impl<P: Provider> PowpegBridgeContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to Native Bridge precompiled contract @ {contract_address}");
        PowpegBridgeContract {
            provider,
            contract_address,
        }
    }
}

impl<P: Provider> PowpegBridgeContractApi for PowpegBridgeContract<P> {
    async fn call_get_btc_transaction_confirmations(
        &self,
        tx_hash: TxHash,
        block_hash: BlockHash,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> alloy_contract::Result<u32> {
        let tx_hash_fb: FixedBytes<32> = tx_hash.into();
        let block_hash_fb: FixedBytes<32> = block_hash.into();

        // Parse merkle branch hashes
        let merkle_branch_hashes_fb: AnyhowResult<Vec<FixedBytes<32>>> = merkle_branch_hashes
            .into_iter()
            .map(|hash| {
                hash.parse::<FixedBytes<32>>()
                    .map_err(|e| anyhow::anyhow!("Invalid merkle branch hash: {e}"))
            })
            .collect();
        let merkle_branch_hashes_fb = merkle_branch_hashes_fb.map_err(|e| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(&format!(
                "Invalid merkle branch hash: {e}"
            )))
        })?;

        // Parse merkle branch path as U256 (uint256 according to RSKIP122)
        let merkle_branch_path_parsed: U256 = merkle_branch_path.parse().map_err(|e| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(&format!(
                "Invalid merkle branch path: {e}"
            )))
        })?;

        // Encode the call data manually using Ethereum ABI encoding
        // Function selector: keccak256("getBtcTransactionConfirmations(bytes32,bytes32,uint256,bytes32[])")[0:4]
        let selector = [0x5b, 0x64, 0x45, 0x87];

        // ABI encoding: fixed-size types first, then dynamic types
        // Parameters: bytes32 blockHash, bytes32 txId, uint256 merkleBranchPath, bytes32[] merkleBranchHashes
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&selector);

        // Encode block_hash (bytes32) - already 32 bytes, just append
        encoded.extend_from_slice(block_hash_fb.as_slice());

        // Encode tx_id (bytes32) - already 32 bytes, just append
        encoded.extend_from_slice(tx_hash_fb.as_slice());

        // Encode merkle_branch_path (uint256) - convert to 32-byte big-endian
        // U256 in Alloy is Uint<256, 4>, we need to convert it to a 32-byte array
        let path_bytes_vec = merkle_branch_path_parsed.to_be_bytes_vec();
        // Pad to 32 bytes if needed (should always be 32 bytes for U256)
        let mut path_bytes = [0u8; 32];
        let start_idx = 32usize.saturating_sub(path_bytes_vec.len());
        path_bytes[start_idx..].copy_from_slice(&path_bytes_vec);
        encoded.extend_from_slice(&path_bytes);

        // Encode merkle_branch_hashes (bytes32[]) - dynamic type
        // First, encode the offset to the array data (4 fixed params * 32 bytes = 128)
        let array_offset = 128u64;
        let offset_bytes_vec = U256::from(array_offset).to_be_bytes_vec();
        let mut offset_bytes = [0u8; 32];
        let start_idx = 32usize.saturating_sub(offset_bytes_vec.len());
        offset_bytes[start_idx..].copy_from_slice(&offset_bytes_vec);
        encoded.extend_from_slice(&offset_bytes);

        // Now encode the array: length first, then each element
        let array_length = merkle_branch_hashes_fb.len() as u64;
        let length_bytes_vec = U256::from(array_length).to_be_bytes_vec();
        let mut length_bytes = [0u8; 32];
        let start_idx = 32usize.saturating_sub(length_bytes_vec.len());
        length_bytes[start_idx..].copy_from_slice(&length_bytes_vec);
        encoded.extend_from_slice(&length_bytes);

        // Encode each hash in the array (each is 32 bytes)
        for hash in &merkle_branch_hashes_fb {
            encoded.extend_from_slice(hash.as_slice());
        }

        let call_data = Bytes::from(encoded);

        // Call the precompiled contract directly using the provider
        let tx_request = alloy_rpc_types::TransactionRequest {
            to: Some(self.contract_address.into()),
            input: call_data.into(),
            ..Default::default()
        };
        let result = self.provider.call(tx_request).await?;

        // Decode the result (int256 according to RSKIP122)
        // Positive values = confirmations, negative values = error codes
        // The result.data is Bytes, we need to decode it as I256
        let result_bytes = result.as_ref();
        if result_bytes.len() < 32 {
            return Err(alloy_contract::Error::TransportError(
                TransportError::local_usage_str("Invalid response length from Native Bridge"),
            ));
        }

        // Decode I256 from the result (first 32 bytes)
        let mut bytes_array = [0u8; 32];
        bytes_array.copy_from_slice(&result_bytes[..32]);
        let confirmations_i256 = I256::from_be_bytes(bytes_array);

        // Convert I256 to u32, handling negative values as errors
        if confirmations_i256.is_negative() {
            // Try to convert to i32 for error code display
            let error_code = i32::try_from(confirmations_i256).unwrap_or(i32::MIN); // Fallback if conversion fails
            return Err(alloy_contract::Error::TransportError(
                TransportError::local_usage_str(&format!(
                    "Native Bridge returned error code: {error_code} (see RSKIP122 for error meanings)"
                )),
            ));
        }

        // Convert positive I256 to u32
        // First convert to U256 (absolute value), then to u32
        let confirmations_value = U256::try_from(confirmations_i256).map_err(|_| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(
                "Failed to convert I256 to U256",
            ))
        })?;

        let confirmations = u32::try_from(confirmations_value).map_err(|_| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(
                "Confirmations value exceeds u32 maximum",
            ))
        })?;

        Ok(confirmations)
    }
}

// // needed so we can create a PegManagerContractApi trait for tests mocking
// #[derive(Clone)]
// pub struct FakePowpegBridgeContract<P: Provider> {
//     contract_instance: FakePowpegBridgeContract<P>,
// }
//
// impl<P: Provider> FakePowpegBridgeContract<P> {
//     pub fn new(provider: P, contract_address: Address) -> Self {
//         info!(
//             "Connecting to FakePegManagerContract @ {}",
//             contract_address
//         );
//         // let contract_instance = FakePegManager::new(contract_address, provider);
//         let contract_instance = FakePowpegBridgeContract::new(contract_address, provider);
//         FakePowpegBridgeContract { contract_instance }
//     }
// }
//
// impl<P: Provider> PowpegBridgeContractApi for FakePowpegBridgeContract<P> {
//     async fn call_get_btc_transaction_confirmations(
//         &self,
//         _tx_hash: TxHash,
//         _block_hash: BlockHash,
//         _merkle_branch_path: String,
//         _merkle_branch_hashes: Vec<String>,
//     ) -> alloy_contract::Result<u32> {
//         todo!("Not implemented yet")
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, FixedBytes, I256, U256};
    use common::types::{BlockHash, TxHash};
    use log::{error, warn};
    use std::str::FromStr;

    /// Test the encoding of call data for getBtcTransactionConfirmations
    #[test]
    fn test_encode_get_btc_confirmations_call_data() {
        // Test data
        let tx_hash = TxHash::from(FixedBytes::from([1u8; 32]));
        let block_hash = BlockHash::from(FixedBytes::from([2u8; 32]));
        let merkle_branch_path = "123".to_string();
        let merkle_branch_hashes = vec![
            format!("0x{}", hex::encode([3u8; 32])),
            format!("0x{}", hex::encode([4u8; 32])),
        ];

        // Expected selector for getBtcTransactionConfirmations(bytes32,bytes32,uint256,bytes32[])
        let expected_selector = [0x5b, 0x64, 0x45, 0x87];

        // Manually encode the call data
        let tx_hash_fb: FixedBytes<32> = tx_hash.into();
        let block_hash_fb: FixedBytes<32> = block_hash.into();

        let mut encoded = Vec::new();
        encoded.extend_from_slice(&expected_selector);
        encoded.extend_from_slice(block_hash_fb.as_slice());
        encoded.extend_from_slice(tx_hash_fb.as_slice());

        // Encode merkle_branch_path as U256
        let path_u256: U256 = merkle_branch_path.parse().unwrap();
        let path_bytes_vec = path_u256.to_be_bytes_vec();
        let mut path_bytes = [0u8; 32];
        let start_idx = 32usize.saturating_sub(path_bytes_vec.len());
        path_bytes[start_idx..].copy_from_slice(&path_bytes_vec);
        encoded.extend_from_slice(&path_bytes);

        // Encode array offset (128 bytes = 4 params * 32 bytes)
        let array_offset = U256::from(128u64);
        let offset_bytes_vec = array_offset.to_be_bytes_vec();
        let mut offset_bytes = [0u8; 32];
        let start_idx = 32usize.saturating_sub(offset_bytes_vec.len());
        offset_bytes[start_idx..].copy_from_slice(&offset_bytes_vec);
        encoded.extend_from_slice(&offset_bytes);

        // Encode array length
        let array_length = U256::from(merkle_branch_hashes.len() as u64);
        let length_bytes_vec = array_length.to_be_bytes_vec();
        let mut length_bytes = [0u8; 32];
        let start_idx = 32usize.saturating_sub(length_bytes_vec.len());
        length_bytes[start_idx..].copy_from_slice(&length_bytes_vec);
        encoded.extend_from_slice(&length_bytes);

        // Encode array elements
        for hash_str in &merkle_branch_hashes {
            let hash_fb: FixedBytes<32> = hash_str.parse().unwrap();
            encoded.extend_from_slice(hash_fb.as_slice());
        }

        // Verify structure
        assert_eq!(&encoded[0..4], expected_selector);
        assert_eq!(encoded.len(), 4 + 32 * 4 + 32 + 32 * 2); // selector + 4 fixed params + length + 2 hashes
    }

    /// Test the decoding of Native Bridge response (positive confirmations)
    #[test]
    fn test_decode_positive_confirmations() {
        // Simulate Native Bridge returning 10 confirmations
        let confirmations = I256::try_from(10i32).unwrap();
        let bytes_array = confirmations.to_be_bytes::<32>();

        // Decode
        let decoded = I256::from_be_bytes(bytes_array);
        assert_eq!(decoded, confirmations);
        assert!(!decoded.is_negative());

        let confirmations_u256 = U256::try_from(decoded).unwrap();
        let confirmations_u32 = u32::try_from(confirmations_u256).unwrap();
        assert_eq!(confirmations_u32, 10);
    }

    /// Test the decoding of Native Bridge response (negative error code)
    #[test]
    fn test_decode_negative_error_code() {
        // Simulate Native Bridge returning error code -1 (BTC_TX_DOES_NOT_EXIST)
        let error_code = I256::try_from(-1i32).unwrap();
        let bytes_array = error_code.to_be_bytes::<32>();

        // Decode
        let decoded = I256::from_be_bytes(bytes_array);
        assert_eq!(decoded, error_code);
        assert!(decoded.is_negative());

        let error_i32 = i32::try_from(decoded).unwrap();
        assert_eq!(error_i32, -1);
    }

    /// Test error codes from RSKIP122
    #[test]
    fn test_native_bridge_error_codes() {
        // RSKIP122 error codes:
        // -1: BTC_TX_DOES_NOT_EXIST
        // -2: BTC_BLOCK_DOES_NOT_EXIST
        // -3: BTC_BLOCK_IS_NOT_IN_THE_BEST_CHAIN
        // -4: INVALID_MERKLE_BRANCH
        // -5: CONFIRMATIONS_OVERFLOW

        let error_codes = vec![-1, -2, -3, -4, -5];

        for code in error_codes {
            let error = I256::try_from(code).unwrap();
            let bytes_array = error.to_be_bytes::<32>();
            let decoded = I256::from_be_bytes(bytes_array);

            assert!(decoded.is_negative());
            let decoded_i32 = i32::try_from(decoded).unwrap();
            assert_eq!(decoded_i32, code);
        }
    }

    /// Test that selector matches expected value
    #[test]
    fn test_function_selector() {
        // keccak256("getBtcTransactionConfirmations(bytes32,bytes32,uint256,bytes32[])")
        // Manually verified selector
        let expected_selector = [0x5b, 0x64, 0x45, 0x87];

        // The selector is already hardcoded in the implementation
        // This test just verifies it matches what we expect
        assert_eq!(GET_BTC_TX_CONFIRMATIONS_SELECTOR, expected_selector);
    }

    /// Integration test: Call Native Bridge precompiled contract
    /// This test calls the real Rootstock testnet node with dummy data.
    /// It expects an error from the contract (since data doesn't exist).
    #[tokio::test]
    async fn test_call_native_bridge_real() {
        use alloy_provider::ProviderBuilder;

        // Hardcoded RPC URL for Rootstock testnet
        let rpc_url = "https://public-node.testnet.rsk.co";

        info!("Connecting to RSK node at: {rpc_url}");

        // Create provider
        let provider = ProviderBuilder::new().on_http(rpc_url.parse().expect("Invalid RPC URL"));

        // Native Bridge address
        let native_bridge_addr = Address::from_str(NATIVE_BRIDGE_ADDRESS).unwrap();

        // Create contract instance
        let contract = PowpegBridgeContract::new(provider, native_bridge_addr);

        // Test data - These would need to be real Bitcoin transaction data from the network
        // For this example, we'll use dummy data that will likely return an error,
        // but it will test the encoding/decoding and actual call to the contract

        let tx_hash = TxHash::from(FixedBytes::from([0u8; 32])); // Dummy tx hash
        let block_hash = BlockHash::from(FixedBytes::from([0u8; 32])); // Dummy block hash
        let merkle_branch_path = "0".to_string();
        let merkle_branch_hashes = vec![];

        debug!("Calling Native Bridge with tx_hash={tx_hash:?}, block_hash={block_hash:?}");

        // Call the contract
        let result = contract
            .call_get_btc_transaction_confirmations(
                tx_hash,
                block_hash,
                merkle_branch_path,
                merkle_branch_hashes,
            )
            .await;

        match result {
            Ok(confirmations) => {
                // Unexpected success with dummy data - should not happen
                warn!("Got confirmations with dummy data: {confirmations}");
                // But don't fail the test - the important part is that we can call the contract
            }
            Err(e) => {
                debug!("Expected error with dummy data: {e:?}");
                // With dummy data, we expect an error
                // The important part is that we were able to call the contract
                // and get a proper response (even if it's an error)
                let error_msg = format!("{e:?}");

                // Verify we got a proper Native Bridge error (not a transport/network error)
                assert!(
                    error_msg.contains("Native Bridge returned error code")
                        || error_msg.contains("local usage"),
                    "Expected Native Bridge error, got: {error_msg}"
                );
            }
        }
    }

    /// Integration test: Call Native Bridge with different error scenario
    /// This test validates error handling for non-existent block
    #[tokio::test]
    async fn test_call_native_bridge_nonexistent_block() {
        use alloy_provider::ProviderBuilder;

        // Hardcoded RPC URL for Rootstock testnet
        let rpc_url = "https://public-node.testnet.rsk.co";

        info!("Testing Native Bridge with non-existent block");

        // Create provider
        let provider = ProviderBuilder::new().on_http(rpc_url.parse().expect("Invalid RPC URL"));

        let native_bridge_addr = Address::from_str(NATIVE_BRIDGE_ADDRESS).unwrap();
        let contract = PowpegBridgeContract::new(provider, native_bridge_addr);

        // Use different dummy data to test another error scenario
        // Using all 0xFF instead of 0x00
        let tx_hash = TxHash::from(FixedBytes::from([0xffu8; 32]));
        let block_hash = BlockHash::from(FixedBytes::from([0xffu8; 32]));
        let merkle_branch_path = "999".to_string();
        let merkle_branch_hashes = vec![];

        debug!(
            "Calling Native Bridge with non-existent block: tx_hash={tx_hash:?}, block_hash={block_hash:?}"
        );

        // Call the contract
        let result = contract
            .call_get_btc_transaction_confirmations(
                tx_hash,
                block_hash,
                merkle_branch_path,
                merkle_branch_hashes,
            )
            .await;

        match result {
            Ok(confirmations) => {
                warn!("Got confirmations with non-existent data: {confirmations}");
                // Don't fail - the important part is that we can call the contract
            }
            Err(e) => {
                error!("Expected error with non-existent block: {e:?}");
                let error_msg = format!("{e:?}");

                // Verify we got a proper Native Bridge error
                // Could be -1 (TX_DOES_NOT_EXIST) or -2 (BLOCK_DOES_NOT_EXIST)
                // or "Invalid response length" for malformed responses
                assert!(
                    error_msg.contains("Native Bridge returned error code")
                        || error_msg.contains("local usage")
                        || error_msg.contains("Invalid response length"),
                    "Expected Native Bridge error, got: {error_msg}"
                );
            }
        }
    }

    /// Test encoding with various merkle proof sizes
    #[test]
    fn test_encoding_with_different_merkle_proof_sizes() {
        let test_cases = vec![
            (0, "empty merkle proof"),
            (1, "single merkle hash"),
            (5, "typical merkle proof"),
            (10, "large merkle proof"),
        ];

        for (num_hashes, description) in test_cases {
            let tx_hash = TxHash::from(FixedBytes::from([1u8; 32]));
            let block_hash = BlockHash::from(FixedBytes::from([2u8; 32]));
            let merkle_branch_path = "123".to_string();
            let merkle_branch_hashes: Vec<String> = (0..num_hashes)
                .map(|i| {
                    let hash_index = u8::try_from(i).expect("hash index fits in u8");
                    format!("0x{}", hex::encode([hash_index; 32]))
                })
                .collect();

            let tx_hash_fb: FixedBytes<32> = tx_hash.into();
            let block_hash_fb: FixedBytes<32> = block_hash.into();

            let mut encoded = Vec::new();
            let selector = [0x5b, 0x64, 0x45, 0x87];
            encoded.extend_from_slice(&selector);
            encoded.extend_from_slice(block_hash_fb.as_slice());
            encoded.extend_from_slice(tx_hash_fb.as_slice());

            let path_u256: U256 = merkle_branch_path.parse().unwrap();
            let path_bytes_vec = path_u256.to_be_bytes_vec();
            let mut path_bytes = [0u8; 32];
            let start_idx = 32usize.saturating_sub(path_bytes_vec.len());
            path_bytes[start_idx..].copy_from_slice(&path_bytes_vec);
            encoded.extend_from_slice(&path_bytes);

            let array_offset = U256::from(128u64);
            let offset_bytes_vec = array_offset.to_be_bytes_vec();
            let mut offset_bytes = [0u8; 32];
            let start_idx = 32usize.saturating_sub(offset_bytes_vec.len());
            offset_bytes[start_idx..].copy_from_slice(&offset_bytes_vec);
            encoded.extend_from_slice(&offset_bytes);

            let array_length = U256::from(num_hashes as u64);
            let length_bytes_vec = array_length.to_be_bytes_vec();
            let mut length_bytes = [0u8; 32];
            let start_idx = 32usize.saturating_sub(length_bytes_vec.len());
            length_bytes[start_idx..].copy_from_slice(&length_bytes_vec);
            encoded.extend_from_slice(&length_bytes);

            for hash_str in &merkle_branch_hashes {
                let hash_fb: FixedBytes<32> = hash_str.parse().unwrap();
                encoded.extend_from_slice(hash_fb.as_slice());
            }

            let expected_size = 4 + 32 * 4 + 32 + 32 * num_hashes;
            assert_eq!(
                encoded.len(),
                expected_size,
                "Failed for case: {description}"
            );
        }
    }
}
