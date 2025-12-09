use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use log::debug;
use primitive_types::{H256, U256};
use sha3::{Digest, Keccak256};

use crate::test_utils::rsk_utils::{UncleBlockInfo, from_hex_to_block_hash, from_hex_to_block_pow};
use crate::types::{BlockDifficulty, BlockHash, BlockNumber, BlockPow, BlockTimestamp, RskBlock};

/// Returns a list of default RSK test blocks.
///
/// This function provides a collection of predefined RSK test blocks, which can be used
/// for testing or reference purposes.
///
/// # Example
///
/// ```
/// use common::test_utils::rsk_block_generator::get_default_rsk_blocks;
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
#[must_use]
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
/// use common::test_utils::rsk_block_generator::get_first_default_rsk_block;
///
/// let block = get_first_default_rsk_block();
/// assert_eq!(block.number(), 7_234_706);
/// ```
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,706](https://explorer.rootstock.io/block/7234706)
#[must_use]
pub fn get_first_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_234_706.into(),
        from_hex_to_block_hash(
            "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c",
        ),
        from_hex_to_block_hash(
            "0x2dbe5baab546a1d1a6c443836810c89867efac727a0b58b24de1baeb15467752",
        ),
        1_739_358_639.into(),
        BlockDifficulty::from(U256::from(10_000_000_000_000_000_000_000_u128)), // difficulty (10 ZH)
        BlockDifficulty::from(U256::from(26_000_000_000_000_000_000_000_000_u128)), // total difficulty (26,000 YH)
        from_hex_to_block_pow(
            "0x0040f824fcc532c20c04a1fc5d66d2dffcbd37742346469195b900000000000000000000407a7f3cbe06b4f6d6b2ddb24bba54202d27d3e44163107c952f6a21cea36d88bd81ac6726770217975bb5e0",
        ),
        vec![from_hex_to_block_hash(
            "0x19a46a22882e08e5a9104a887dd66eecd2c71a1e887587f39dbf2d30c2616346",
        )],
    )
}

/// This function returns a second default RSK test block.
///
/// # Example
///
/// ```
/// use common::test_utils::rsk_block_generator::get_second_default_rsk_block;
///
/// let block = get_second_default_rsk_block();
/// assert_eq!(block.number(), 7_234_707);
/// ```
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,707](https://explorer.rootstock.io/block/7234707)
#[must_use]
pub fn get_second_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_234_707.into(),
        from_hex_to_block_hash(
            "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca",
        ),
        from_hex_to_block_hash(
            "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c",
        ),
        1_739_358_657.into(),
        BlockDifficulty::from(U256::from(10_000_000_000_000_000_000_000_u128)), // difficulty (10 ZH)
        BlockDifficulty::from(U256::from(26_000_000_000_000_000_000_000_000_u128)), // total difficulty (26,000 YH)
        from_hex_to_block_pow(
            "0x00a00e20fcc532c20c04a1fc5d66d2dffcbd37742346469195b9000000000000000000003270dc5de9a169bdd5794d6d3f8e8595007a04966bece93015c60bee50e33dc6c581ac6726770217d62abb2a",
        ),
        vec![],
    )
}

/// This function returns a third default RSK test block.
///
/// # Example
///
/// ```
/// use common::test_utils::rsk_block_generator::get_third_default_rsk_block;
///
/// let block = get_third_default_rsk_block();
/// assert_eq!(block.number(), 7_234_708);
/// ```
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,708](https://explorer.rootstock.io/block/7234708)
#[must_use]
pub fn get_third_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_234_708.into(),
        from_hex_to_block_hash(
            "0x9971862c7475888178eae1e2cd03dde72e3791ddd72853a8f781022a49a95228",
        ),
        from_hex_to_block_hash(
            "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca",
        ),
        1_739_358_667.into(),
        BlockDifficulty::from(U256::from(10_000_000_000_000_000_000_000_u128)), // difficulty (10 ZH)
        BlockDifficulty::from(U256::from(26_000_000_000_000_000_000_000_000_u128)), // total difficulty (26,000 YH)
        from_hex_to_block_pow(
            "0x00400020fcc532c20c04a1fc5d66d2dffcbd37742346469195b90000000000000000000091bd0ea538156c5d96c1163769f7da85f28c3197d482991dbd0f014242805b28dd81ac6726770217387d6c91",
        ),
        vec![from_hex_to_block_hash(
            "0xd257bf689705e7dcdce1174addccc3e7b495ef60040e6de39fafb5da00eff19a",
        )],
    )
}

#[must_use]
pub fn event_signature_to_topic(event_signature: &str) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(event_signature.as_bytes());
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(hash))
}

/// A stateless generator for fake RSK blocks that computes dynamic values (difficulty, timestamp,
/// total difficulty and average block time) based on the block number. It has a built-in mechanism
/// to handle generation of alternative blocks (to simulate reorganizations).
#[derive(Clone)]
pub struct FakeBlockGenerator {
    base_difficulty: BlockDifficulty,
    difficulty_increment: BlockDifficulty,
    base_timestamp: BlockTimestamp,
    avg_block_time: u64,
    reorg_block_height: Option<BlockNumber>,
    is_reorg: Arc<AtomicBool>,
    uncle_block_info_vec: Option<Vec<UncleBlockInfo>>,
}

impl FakeBlockGenerator {
    /// # Panics
    ///
    /// Panics if the difficulty string cannot be parsed.
    pub fn new(
        reorg_block_height: Option<BlockNumber>,
        is_reorg: Arc<AtomicBool>,
        uncle_block_info_vec: Option<Vec<UncleBlockInfo>>,
    ) -> Self {
        Self {
            base_difficulty: BlockDifficulty::from(
                U256::from_dec_str("10000000000000000000000").unwrap(),
            ),
            difficulty_increment: BlockDifficulty::from(
                U256::from_dec_str("10000000000000000").unwrap(),
            ),
            base_timestamp: 1_514_980_800.into(),
            avg_block_time: 30,
            reorg_block_height,
            is_reorg,
            uncle_block_info_vec,
        }
    }

    #[must_use]
    pub fn generate_hash(&self, height: BlockNumber, flavor: &str) -> String {
        let mut hasher = Keccak256::new();
        let bytes = if flavor.is_empty() {
            height.value().to_le_bytes().to_vec()
        } else {
            height.value().to_le_bytes().iter().chain(flavor.as_bytes()).copied().collect()
        };
        hasher.update(&bytes);
        let result = hasher.finalize();
        format!("0x{result:064x}")
    }

    /// Generates a fake RSK block for the given block height.
    #[must_use]
    pub fn generate_block(
        &self,
        height: BlockNumber,
        uncle_info: Option<&UncleBlockInfo>, // uncle_info is None for regular blocks, Some for uncle blocks
    ) -> Option<RskBlock> {
        let is_reorg = self.is_reorg.load(Ordering::SeqCst);
        let reorged_block = is_reorg && self.reorg_block_height.is_some_and(|h| height > h);
        if uncle_info.as_ref().is_some_and(|info| info.reorg != reorged_block) {
            return None; // do not generate an uncle block if uncle_info.reorg does not match current reorg status
        }
        let parent_hash = self.generate_parent_hash(height, self.reorg_block_height, is_reorg);
        let block_hash = self.generate_block_hash(height, is_reorg, uncle_info);
        debug!(
            "Generating block {} with hash: {} -- parent hash: {} -- is_reorg: {}{}",
            height,
            block_hash,
            parent_hash,
            is_reorg,
            if let Some(uncle_info) = &uncle_info {
                format!(" -- uncle_id: {}", uncle_info.id)
            } else {
                String::new()
            }
        );
        let uncles_vec = if uncle_info.is_some() {
            vec![] // if uncle_info is provided, this is an uncle block, so no uncles
        } else {
            self.generate_uncles_vec(height, is_reorg)
        };
        let diff = self.generate_difficulty(height);
        let tot_diff = self.generate_total_difficulty(height);
        let ts = self.generate_timestamp(height);
        let block_pow = {
            let mut hasher = Keccak256::new();
            hasher.update(block_hash.value().as_bytes());
            hasher.update(parent_hash.value().as_bytes());
            hasher.update(ts.value().to_le_bytes());
            BlockPow::from(H256::from_slice(&hasher.finalize()))
        };
        Some(RskBlock::new(
            height,
            block_hash,
            parent_hash,
            ts,
            diff,
            tot_diff,
            block_pow,
            uncles_vec,
        ))
    }

    fn generate_uncles_vec(&self, height: BlockNumber, is_reorg: bool) -> Vec<BlockHash> {
        let mut uncles_vec: Vec<BlockHash> = vec![];
        if let Some(uncle_block_info_vec) = &self.uncle_block_info_vec {
            let reorged_block = is_reorg && self.reorg_block_height.is_some_and(|h| height > h);
            for uncle_info in uncle_block_info_vec {
                if uncle_info.height == height && uncle_info.reorg == reorged_block {
                    let flavor = format!(
                        "uncle_{}{}",
                        if reorged_block { "alt" } else { "" },
                        uncle_info.id
                    );
                    let uncle_hash = from_hex_to_block_hash(&self.generate_hash(height, &flavor));
                    debug!("Adding uncle {} to block {}", uncle_info.id, height);
                    uncles_vec.push(uncle_hash);
                }
            }
        }
        uncles_vec
    }

    fn generate_parent_hash(
        &self,
        height: BlockNumber,
        reorg_block_height: Option<BlockNumber>,
        is_reorg: bool,
    ) -> BlockHash {
        let parent_hash = if height == 0 {
            "0x0000000000000000000000000000000000000000000000000000000000000000".to_string()
        } else {
            let flavor: String = if reorg_block_height.is_some_and(|h| height > h) && is_reorg {
                "alt".to_string()
            } else {
                String::new()
            };
            self.generate_hash(height - 1, &flavor)
        };
        from_hex_to_block_hash(&parent_hash)
    }

    fn generate_block_hash(
        &self,
        height: BlockNumber,
        is_reorg: bool,
        uncle_info: Option<&UncleBlockInfo>,
    ) -> BlockHash {
        let reorged_block = is_reorg && self.reorg_block_height.is_some_and(|h| height > h);
        let flavor = if let Some(uncle_info) = uncle_info {
            format!("uncle_{}{}", if reorged_block { "alt" } else { "" }, uncle_info.id)
        } else {
            (if reorged_block { "alt" } else { "" }).to_string()
        };
        from_hex_to_block_hash(&self.generate_hash(height, &flavor))
    }

    fn generate_difficulty(&self, height: BlockNumber) -> BlockDifficulty {
        let diff = self.base_difficulty.value()
            + U256::from(height.value()) * self.difficulty_increment.value();
        BlockDifficulty::from(diff)
    }

    fn generate_timestamp(&self, height: BlockNumber) -> BlockTimestamp {
        BlockTimestamp::from(self.base_timestamp.value() + height.value() * self.avg_block_time)
    }

    fn generate_total_difficulty(&self, height: BlockNumber) -> BlockDifficulty {
        let n = U256::from(height.value());
        let sum_n = n * (n + U256::one()) / U256::from(2u32);
        let total_diff =
            n * self.base_difficulty.value() + self.difficulty_increment.value() * sum_n;
        BlockDifficulty::from(total_diff)
    }
}

#[must_use]
pub fn create_block_and_uncles() -> (RskBlock, RskBlock, RskBlock) {
    let block_1_template = get_first_default_rsk_block();

    let block_1 = create_block_from_template(
        &block_1_template,
        "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
        block_1_template.parent_hash(),
        vec![],
    );

    let uncle_1 = create_block_from_template(
        &block_1_template,
        "0x3e5f9c2451b8efb4c1e3739816e44e4f0e9c25b2f9f6a57bdbf71e2df7c1b790",
        block_1_template.parent_hash(),
        vec![],
    );

    let block_2 = create_block_from_template(
        &get_second_default_rsk_block(),
        "0x5c8a91d7ef0d46f3a65f1c345beab0cf56a8e065f2b762fe9b8e2d771fd42c83",
        block_1.hash(),
        vec![uncle_1.hash()],
    );

    (block_1, uncle_1, block_2)
}

/// # Panics
///
/// Panics if the hash cannot be parsed.
#[must_use]
pub fn create_block_from_template(
    template: &RskBlock,
    hash: &str,
    parent: BlockHash,
    uncles: Vec<BlockHash>,
) -> RskBlock {
    RskBlock::new(
        template.number(),
        BlockHash::try_from(hash).expect("Failed to parse hash"),
        parent,
        template.timestamp(),
        template.difficulty(),
        template.total_difficulty(),
        template.pow(),
        uncles,
    )
}
