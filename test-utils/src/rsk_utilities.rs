use common::types::RskBlock;
use primitive_types::U256;
use sha3::{Digest, Keccak256};

/// Returns a list of default RSK test blocks.
///
/// This function provides a collection of predefined RSK test blocks, which can be used
/// for testing or reference purposes.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utilities::get_default_rsk_blocks;
///
/// let blocks = get_default_rsk_blocks();
///
/// assert_eq!(blocks.len(), 3);
/// assert_eq!(blocks[0].number(), 7_234_706);
/// assert_eq!(blocks[1].number(), 7_234_707);
/// assert_eq!(blocks[2].number(), 7_234_708);
/// ```
///
/// # Returns
///
/// A `Vec<RskBlock>` containing three default RSK blocks.
pub fn get_default_rsk_blocks() -> Vec<RskBlock> {
    vec![
        get_first_default_rsk_block(),
        get_second_default_rsk_block(),
        get_third_default_rsk_block(),
    ]
}

/// This function returns a first default RSK test block.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utilities::get_first_default_rsk_block;
///
/// let block = get_first_default_rsk_block();
/// assert_eq!(block.number(), 7_234_706);
/// ```
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,706](https://explorer.rootstock.io/block/7234706)
pub fn get_first_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_234_706,
        "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c".to_string(),
        "0x2dbe5baab546a1d1a6c443836810c89867efac727a0b58b24de1baeb15467752".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358639,
        "0xcc018a4152524f57484541442d".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// This function returns a second default RSK test block.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utilities::get_second_default_rsk_block;
///
/// let block = get_second_default_rsk_block();
/// assert_eq!(block.number(), 7_234_707);
/// ```
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,707](https://explorer.rootstock.io/block/7234707)
pub fn get_second_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_234_707,
        "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca".to_string(),
        "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358657,
        "pow_string".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// This function returns a third default RSK test block.
///
/// # Example
///
/// ```
/// use test_utils::rsk_utilities::get_third_default_rsk_block;
///
/// let block = get_third_default_rsk_block();
/// assert_eq!(block.number(), 7_234_708);
/// ```
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,708](https://explorer.rootstock.io/block/7234708)
pub fn get_third_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_234_708,
        "0x9971862c7475888178eae1e2cd03dde72e3791ddd72853a8f781022a49a95228".to_string(),
        "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358667,
        "pow_string".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

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

pub fn get_fake_tx_hash(tx_id: u64, from: &str) -> String {
    let mut hasher = Keccak256::new();
    let mut data = Vec::new();
    data.extend_from_slice(&tx_id.to_le_bytes());
    data.extend_from_slice(from.as_bytes());
    hasher.update(data);
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(hash))
}

pub fn address_to_topic(address: &str) -> String {
    let addr = address.strip_prefix("0x").unwrap_or(address);
    if addr.len() != 40 {
        panic!(
            "Invalid Rootstock address length: expected 40 hex digits, got {}",
            addr.len()
        );
    }
    format!("0x{}{}", "0".repeat(24), addr)
}

pub fn event_signature_to_topic(event_signature: &str) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(event_signature.as_bytes());
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(hash))
}
