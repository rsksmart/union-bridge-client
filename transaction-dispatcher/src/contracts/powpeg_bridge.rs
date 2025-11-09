use alloy_primitives::{Address, Bytes, FixedBytes, I256, U256};
use alloy_provider::Provider;
use alloy_sol_types::SolCall;
use alloy_transport::TransportError;
use anyhow::Result as AnyhowResult;
use common::types::{BlockHash, TxHash};
use log::info;

// re-export for convenience
use actors_mocking::fake_contracts::FakePegManager;
use actors_mocking::fake_contracts::FakePegManager::FakePegManagerInstance;
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
        info!(
            "Connecting to Native Bridge precompiled contract @ {}",
            contract_address
        );
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
                    .map_err(|e| anyhow::anyhow!("Invalid merkle branch hash: {}", e))
            })
            .collect();
        let merkle_branch_hashes_fb = merkle_branch_hashes_fb.map_err(|e| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(&format!(
                "Invalid merkle branch hash: {}",
                e
            )))
        })?;

        // Parse merkle branch path as U256 (uint256 according to RSKIP122)
        let merkle_branch_path_parsed: U256 = merkle_branch_path.parse().map_err(|e| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(&format!(
                "Invalid merkle branch path: {}",
                e
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
                    "Native Bridge returned error code: {} (see RSKIP122 for error meanings)",
                    error_code
                )),
            ));
        }

        // Convert positive I256 to u32
        // First convert to U256 (absolute value), then to u32
        let confirmations_u256 = U256::try_from(confirmations_i256).map_err(|_| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(
                "Failed to convert I256 to U256",
            ))
        })?;

        let confirmations = u32::try_from(confirmations_u256).map_err(|_| {
            alloy_contract::Error::TransportError(TransportError::local_usage_str(
                "Confirmations value exceeds u32 maximum",
            ))
        })?;

        Ok(confirmations)
    }
}

// needed so we can create a PegManagerContractApi trait for tests mocking
#[derive(Clone)]
pub struct FakePowpegBridgeContract<P: Provider> {
    contract_instance: FakePegManagerInstance<P>,
}

impl<P: Provider> FakePowpegBridgeContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!(
            "Connecting to FakePegManagerContract @ {}",
            contract_address
        );
        let contract_instance = FakePegManager::new(contract_address, provider);
        FakePowpegBridgeContract { contract_instance }
    }
}

impl<P: Provider> PowpegBridgeContractApi for FakePowpegBridgeContract<P> {
    async fn call_get_btc_transaction_confirmations(
        &self,
        tx_hash: TxHash,
        block_hash: BlockHash,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> alloy_contract::Result<u32> {
        todo!("Not implemented yet")
    }
}

// pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
//     let decoded_err = err.as_decoded_interface_error::<PowpegBrdigeErrors>();
//     decoded_err.map(|e| match e {
//         PegManagerErrors::PeginAlreadyAccepted(e) => {
//             DomainErrors::PeginAlreadyAccepted(format!("{:?}", e))
//         }
//         PegManagerErrors::PeginAlreadyRequested(e) => {
//             DomainErrors::PeginAlreadyRequested(format!("{:?}", e))
//         }
//         PegManagerErrors::IncorrectInputsNumber(e) => {
//             DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
//         }
//         PegManagerErrors::IncorrectOutputsNumber(e) => {
//             DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
//         }
//         PegManagerErrors::InvalidBtcTxVersion(e) => {
//             DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
//         }
//         PegManagerErrors::InvalidLocktime(e) => {
//             DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
//         }
//         PegManagerErrors::InvalidCompressedPubKey(e) => {
//             DomainErrors::InvalidCompressedPubKey(format!("{:?}", e))
//         }
//         PegManagerErrors::PegoutRequestAmountExceedsUint64Limit(e) => {
//             DomainErrors::PegoutRequestAmountExceedsUint64Limit(format!("{:?}", e))
//         }
//         // Native Bridge Errors
//         PegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
//             // we consider this reversible, so we map it to MissingConfirmationsOnNativeBridge
//             DomainErrors::MissingConfirmationsOnNativeBridge(format!("{:?}", e))
//         }
//         PegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
//             DomainErrors::MissingConfirmationsOnNativeBridge(format!("{:?}", e))
//         }
//         PegManagerErrors::NotEnoughConfirmations(e) => {
//             DomainErrors::MissingConfirmationsOnNativeBridge(format!("{:?}", e))
//         }
//         PegManagerErrors::InvalidSlotState(e) => {
//             // Extract actual and expected values from the error
//             // The InvalidSlotState error typically contains actual and expected slot states
//             DomainErrors::InvalidSlotState {
//                 expected: e.expected as u8,
//                 actual: e.actual as u8,
//             }
//         }
//         // Unhandled
//         _ => DomainErrors::UnhandledContractError(format!("{:?}", e)),
//     })
// }
//
// // todo(fede) add tests
