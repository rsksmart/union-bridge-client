use common::types::{Address, BlockHash, BlockPow, ContractInfo};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

pub const DEFAULT_ADDRESS: &str = "0x5a23bef6b2051Fc91a8b9B0307ed08D09C07cc2d";
pub const DEFAULT_BLOCK_HASH: &str =
    "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c";
pub const DEFAULT_BITCOIN_MERGED_MINING_HEADER: &str =
    "0x00000020538fb0d4d0cbdf0f3b88e02551018fcd6064cbe5cbed40d78b4c3709000000004feaeec0d7a118f6d1c0d8fec32936b9dfff3bea45b537027c6439ac5ea98ccd34b8b467908316194c8b4487";

/// Generates a fake Rootstock address based on a given number.
///
/// This function computes a Keccak256 hash of the little-endian byte representation
/// of `address_num` appended with the optional nonce (if provided). The last 20 bytes
/// of the hash are then formatted as a hexadecimal string to simulate a Rootstock address.
///
/// # Parameters
///
/// - `address_num`: A numeric identifier used as part of the address generation.
///
/// # Returns
///
/// An `Address` representing the fake Rootstock address.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utils::generate_fake_address;
///
/// let address = generate_fake_address(1);
/// ```
pub fn generate_fake_address(address_num: u64) -> Address {
    let mut hasher = Keccak256::new();
    let data = address_num.to_le_bytes().to_vec();
    // Append nonce bytes if provided
    hasher.update(data);
    let hash = hasher.finalize();
    // Rootstock addresses are the last 20 bytes of the 32-byte hash
    let address_bytes = &hash[12..];
    let addr = format!("0x{}", hex::encode(address_bytes));
    Address::try_from(addr.as_str()).unwrap()
}

pub fn generate_fake_addresses(addresses_size: u64) -> Vec<Address> {
    (0..addresses_size)
        .map(|i| generate_fake_address(i))
        .collect()
}

pub fn generate_fake_managed_contracts(addresses: Vec<Address>) -> HashMap<Address, ContractInfo> {
    addresses
        .into_iter()
        .map(|address| generate_fake_managed_contract(address))
        .collect()
}

pub fn generate_fake_managed_contract(address: Address) -> (Address, ContractInfo) {
    (
        address,
        ContractInfo {
            name: format!("contract_{}", address.to_string()),
            address,
            abi: None,
        },
    )
}

/// Generates a fake transaction hash using a transaction ID and a sender address.
///
/// This function concatenates the little-endian byte representation of `tx_id` with the
/// bytes of the `from` string, computes the Keccak256 hash of the result, and returns
/// the hash formatted as a hexadecimal string.
///
/// # Parameters
///
/// - `tx_id`: The transaction identifier used in the hash generation.
/// - `from`: A string slice representing the sender's address.
///
/// # Returns
///
/// A `String` containing the fake transaction hash.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utils::generate_fake_tx_hash;
///
/// let tx_hash = generate_fake_tx_hash(1, "0xabc123...");
/// assert!(tx_hash.starts_with("0x"));
/// ```
pub fn generate_fake_tx_hash(tx_id: u64, from: &str) -> String {
    let mut hasher = Keccak256::new();
    let mut data = Vec::new();
    data.extend_from_slice(&tx_id.to_le_bytes());
    data.extend_from_slice(from.as_bytes());
    hasher.update(data);
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(hash))
}

/// Converts a hex string into a `BlockHash`.
///
/// # Panics
///
/// This function will panic if the string is not a valid hexadecimal.
/// ```
pub fn from_hex_to_block_hash(hex: &str) -> BlockHash {
    BlockHash::try_from(hex).expect(&format!("Invalid hex string: {}", hex))
}

/// Converts a Bitcoin merged mining hex string into a `BlockPow`.
///
/// # Panics
///
/// This function will panic if the string is not a valid hexadecimal.
/// ```
pub fn from_hex_to_block_pow(hex: &str) -> BlockPow {
    BlockPow::try_from(hex).expect(&format!("Invalid hex string: {}", hex))
}
