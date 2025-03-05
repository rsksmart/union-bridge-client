use common::types::RskBlock;
use log::debug;
use primitive_types::U256;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

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
        7_234_708,
        "0x9971862c7475888178eae1e2cd03dde72e3791ddd72853a8f781022a49a95228".to_string(),
        "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358667,
        "pow_string".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// A stateless generator for fake RSK blocks that computes dynamic values (difficulty, timestamp,
/// total difficulty and average block time) based on the block number. It has a built-in mechanism
/// to handle generation of alternative blocks (to simulate reorganizations).`.
#[derive(Clone)]
pub struct FakeBlockGenerator {
    base_difficulty: U256,
    difficulty_increment: U256,
    base_timestamp: i64,
    avg_block_time: i64,
    reorg_block_height: u64,
    is_reorg: Arc<AtomicBool>,
}

impl FakeBlockGenerator {
    pub fn new(reorg_block_height: u64, is_reorg: Arc<AtomicBool>) -> Self {
        Self {
            base_difficulty: U256::from_dec_str("10000000000000000000000").unwrap(),
            difficulty_increment: U256::from_dec_str("10000000000000000").unwrap(),
            base_timestamp: 1514980800,
            avg_block_time: 30,
            reorg_block_height,
            is_reorg,
        }
    }

    pub fn generate_hash(&self, height: u64, flavor: &str) -> String {
        let mut hasher = Sha256::new();
        let bytes = if flavor.is_empty() {
            height.to_le_bytes().to_vec()
        } else {
            height
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
    pub fn generate_block(&self, height: u64) -> RskBlock {
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
        let block_hash = self.generate_hash(
            height,
            if (height >= self.reorg_block_height) && is_reorg {
                "alt"
            } else {
                ""
            },
        );
        debug!(
            "Generating block {} with hash: {} -- parent hash: {} -- is_reorg: {}",
            height, block_hash, parent_hash, is_reorg
        );
        let diff = self.generate_difficulty(height);
        let tot_diff = self.generate_total_difficulty(height);
        let ts = self.generate_timestamp(height);
        RskBlock::new(
            height,
            block_hash,
            parent_hash,
            diff,
            ts as u64,
            "pow_string".to_string(),
            tot_diff,
        )
    }

    fn generate_difficulty(&self, height: u64) -> U256 {
        self.base_difficulty + U256::from(height) * self.difficulty_increment
    }

    fn generate_timestamp(&self, height: u64) -> i64 {
        self.base_timestamp + (height as i64) * self.avg_block_time
    }

    fn generate_total_difficulty(&self, height: u64) -> U256 {
        let n = U256::from(height);
        let sum_n = n * (n + U256::one()) / U256::from(2u32);
        n * self.base_difficulty + self.difficulty_increment * sum_n
    }
}
