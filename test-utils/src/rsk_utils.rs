use common::types::BlockHash;
use sha3::{Digest, Keccak256};

pub const DEFAULT_BLOCK_HASH: &str =
    "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c";

/// Generates a fake Rootstock address based on a given number and an optional nonce.
///
/// This function computes a Keccak256 hash of the little-endian byte representation
/// of `address_num` appended with the optional nonce (if provided). The last 20 bytes
/// of the hash are then formatted as a hexadecimal string to simulate a Rootstock address.
///
/// # Parameters
///
/// - `address_num`: A numeric identifier used as part of the address generation.
/// - `nonce`: An optional string slice to differentiate addresses.
///
/// # Returns
///
/// A `String` representing the fake Rootstock address.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utils::get_fake_address;
///
/// let address = get_fake_address(1, Some("nonce"));
/// assert!(address.starts_with("0x"));
/// ```
pub fn get_fake_address(address_num: u64, nonce: Option<&str>) -> String {
    let mut hasher = Keccak256::new();
    let mut data = address_num.to_le_bytes().to_vec();
    // Append nonce bytes if provided
    if let Some(n) = nonce {
        data.extend_from_slice(n.as_bytes());
    }
    hasher.update(data);
    let hash = hasher.finalize();
    // Rootstock addresses are the last 20 bytes of the 32-byte hash
    let address_bytes = &hash[12..];
    format!("0x{}", hex::encode(address_bytes))
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
/// use test_utils::rsk_utils::get_fake_tx_hash;
///
/// let tx_hash = get_fake_tx_hash(1, "0xabc123...");
/// assert!(tx_hash.starts_with("0x"));
/// ```
pub fn get_fake_tx_hash(tx_id: u64, from: &str) -> String {
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
///
/// # Examples
///
/// ```
/// use common::types::BlockHash;
/// use test_utils::rsk_utils::from_hex_to_block_hash;
///
/// // Valid usage:
/// let valid_hex = "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c";
/// let block_hash = from_hex_to_block_hash(valid_hex);
/// assert_eq!(block_hash.to_string(), valid_hex);
/// ```
///
/// ```should_panic
/// // This will panic because it's invalid hexadecimal:
/// use test_utils::rsk_utils::from_hex_to_block_hash;
/// from_hex_to_block_hash("not-valid-hex");
/// ```
pub fn from_hex_to_block_hash(hex: &str) -> BlockHash {
    BlockHash::try_from(hex).expect(&format!("Invalid hex string: {}", hex))
}
