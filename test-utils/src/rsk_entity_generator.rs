use common::types::{BlockHash, BlockNumber, BlockTimestamp, RskBlock};
use log::debug;
use primitive_types::U256;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use common::types::{LogEvent, LogInfo, RskBlock, RskLog};
use primitive_types::U256;
use sha3::{Digest, Keccak256};

pub const DEFAULT_BLOCK_HASH: &str =
    "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c";
/// Returns a list of default RSK test blocks.
///
/// This function provides a collection of predefined RSK test blocks, which can be used
/// for testing or reference purposes.
///
/// # Example
///
/// ```
/// use test_utils::rsk_entity_generator::get_default_rsk_blocks;
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
/// use test_utils::rsk_entity_generator::get_first_default_rsk_block;
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
        7_234_706.into(),
        from_hex_to_block_hash(
            "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c",
        ),
        from_hex_to_block_hash(
            "0x2dbe5baab546a1d1a6c443836810c89867efac727a0b58b24de1baeb15467752",
        ),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358639.into(),
        "0xcc018a4152524f57484541442d".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// This function returns a second default RSK test block.
///
/// # Example
///
/// ```
/// use test_utils::rsk_entity_generator::get_second_default_rsk_block;
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
        7_234_707.into(),
        from_hex_to_block_hash(
            "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca",
        ),
        from_hex_to_block_hash(
            "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c",
        ),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358657.into(),
        "pow_string".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// This function returns a third default RSK test block.
///
/// # Example
///
/// ```
/// use test_utils::rsk_entity_generator::get_third_default_rsk_block;
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
        7_234_708.into(),
        from_hex_to_block_hash(
            "0x9971862c7475888178eae1e2cd03dde72e3791ddd72853a8f781022a49a95228",
        ),
        from_hex_to_block_hash(
            "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca",
        ),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358667.into(),
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
    // Ethereum addresses are the last 20 bytes of the 32-byte hash
    let address_bytes = &hash[12..];
    format!("0x{}", hex::encode(address_bytes))
}

pub fn get_fake_tx_hash(tx_id: u64, from: &str) -> String {
    let mut hasher = Keccak256::new();
    let mut data = Vec::new();
    data.extend_from_slice(&tx_id.to_le_bytes());
    data.extend_from_slice(from.as_bytes());
    // data.extend_from_slice(to.as_bytes());
    // data.extend_from_slice(&value.to_le_bytes());
    hasher.update(data);
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(hash))
}

pub fn address_to_topic(address: &str) -> String {
    let addr = address.strip_prefix("0x").unwrap_or(address);
    if addr.len() != 40 {
        panic!(
            "Invalid Ethereum address length: expected 40 hex digits, got {}",
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

/// A stateless generator for fake RSK blocks that computes dynamic values (difficulty, timestamp,
/// total difficulty and average block time) based on the block number. It has a built-in mechanism
/// to handle generation of alternative blocks (to simulate reorganizations).`.
#[derive(Clone)]
pub struct FakeBlockGenerator {
    base_difficulty: U256,
    difficulty_increment: U256,
    base_timestamp: BlockTimestamp,
    avg_block_time: u64,
    reorg_block_height: BlockNumber,
    is_reorg: Arc<AtomicBool>,
}

impl FakeBlockGenerator {
    pub fn new(reorg_block_height: BlockNumber, is_reorg: Arc<AtomicBool>) -> Self {
        Self {
            base_difficulty: U256::from_dec_str("10000000000000000000000").unwrap(),
            difficulty_increment: U256::from_dec_str("10000000000000000").unwrap(),
            base_timestamp: 1514980800.into(),
            avg_block_time: 30,
            reorg_block_height,
            is_reorg,
        }
    }

    pub fn generate_hash(&self, height: BlockNumber, flavor: &str) -> String {
        let mut hasher = Keccak256::new();
        let bytes = if flavor.is_empty() {
            height.value().to_le_bytes().to_vec()
        } else {
            height
                .value()
                .to_le_bytes()
                .iter()
                .chain(flavor.as_bytes())
                .copied()
                .collect()
        };
        hasher.update(&bytes);
        let result = hasher.finalize();
        format!("0x{:064x}", result)
    }

    /// Generates a fake RSK block for the given block height.
    pub fn generate_block(&self, height: BlockNumber) -> RskBlock {
        let is_reorg = self.is_reorg.load(Ordering::SeqCst);
        let parent_hash = if height == 0 {
            "0x0000000000000000000000000000000000000000000000000000000000000000".to_string()
        } else {
            self.generate_hash(
                height - 1,
                if (height > self.reorg_block_height) && is_reorg {
                    "alt"
                } else {
                    ""
                },
            )
        };
        let parent_hash = from_hex_to_block_hash(&parent_hash);

        let block_hash = self.generate_hash(
            height,
            if (height >= self.reorg_block_height) && is_reorg {
                "alt"
            } else {
                ""
            },
        );
        let block_hash = from_hex_to_block_hash(&block_hash);

        debug!(
            "Generating block {} with hash: {} -- parent hash: {} -- is_reorg: {}",
            height, block_hash, parent_hash, is_reorg
        );
        let diff = self.generate_difficulty(height);
        let tot_diff = self.generate_total_difficulty(height);
        let ts = self.generate_timestamp(height);
        RskBlock::new(
            height.into(),
            block_hash,
            parent_hash,
            diff,
            ts,
            "pow_string".to_string(),
            tot_diff,
        )
    }

    fn generate_difficulty(&self, height: BlockNumber) -> U256 {
        self.base_difficulty + U256::from(height.value()) * self.difficulty_increment
    }

    fn generate_timestamp(&self, height: BlockNumber) -> BlockTimestamp {
        BlockTimestamp::from(self.base_timestamp.value() + height.value() * self.avg_block_time)
    }

    fn generate_total_difficulty(&self, height: BlockNumber) -> U256 {
        let n = U256::from(height.value());
        let sum_n = n * (n + U256::one()) / U256::from(2u32);
        n * self.base_difficulty + self.difficulty_increment * sum_n
    }
}

fn from_hex_to_block_hash(hex: &str) -> BlockHash {
    BlockHash::try_from(hex).expect(&format!("Invalid hex string: {}", hex))
}

/// A stateless generator for fake RSK logs.
#[derive(Clone)]
pub struct FakeLogGenerator {
    event_signature: String,
}

impl FakeLogGenerator {
    pub fn new(event_signature: &str) -> Self {
        Self {
            event_signature: event_signature.to_string(),
        }
    }

    pub fn generate_log(
        &self,
        block: RskBlock,
        tx_id: u64,
        address_num: u64,
        log_index: u64,
    ) -> RskLog {
        let address_from = get_fake_address(address_num, None);
        let address_to = get_fake_address(address_num, Some("destinatary"));
        let tx_hash = get_fake_tx_hash(tx_id, &address_from);
        let info: LogInfo = LogInfo::new(
            address_from.clone(),
            block.hash().to_string(),
            block.number(),
            tx_hash,
            log_index,
            false,
        );
        let topics = vec![
            address_to_topic(&address_from),
            address_to_topic(&address_to),
        ];
        let event: LogEvent =
            LogEvent::new(event_signature_to_topic(&self.event_signature), topics);
        RskLog::new(info, event)
    }
}
