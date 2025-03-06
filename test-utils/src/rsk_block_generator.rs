use common::types::{BlockNumber, RskBlock};
use log::debug;
use primitive_types::U256;
use sha3::{Digest, Keccak256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::rsk_utilities::from_hex_to_block_hash;

/// A stateless generator for fake RSK blocks that computes dynamic values (difficulty, timestamp,
/// total difficulty and average block time) based on the block number. It has a built-in mechanism
/// to handle generation of alternative blocks (to simulate reorganizations).`.
#[derive(Clone)]
pub struct FakeBlockGenerator {
    base_difficulty: U256,
    difficulty_increment: U256,
    base_timestamp: i64,
    avg_block_time: i64,
    reorg_block_height: BlockNumber,
    is_reorg: Arc<AtomicBool>,
}

impl FakeBlockGenerator {
    pub fn new(reorg_block_height: BlockNumber, is_reorg: Arc<AtomicBool>) -> Self {
        Self {
            base_difficulty: U256::from_dec_str("10000000000000000000000").unwrap(),
            difficulty_increment: U256::from_dec_str("10000000000000000").unwrap(),
            base_timestamp: 1514980800,
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
            ts as u64,
            "pow_string".to_string(),
            tot_diff,
        )
    }

    fn generate_difficulty(&self, height: BlockNumber) -> U256 {
        self.base_difficulty + U256::from(height.value()) * self.difficulty_increment
    }

    fn generate_timestamp(&self, height: BlockNumber) -> i64 {
        self.base_timestamp + (height.value() as i64) * self.avg_block_time
    }

    fn generate_total_difficulty(&self, height: BlockNumber) -> U256 {
        let n = U256::from(height.value());
        let sum_n = n * (n + U256::one()) / U256::from(2u32);
        n * self.base_difficulty + self.difficulty_increment * sum_n
    }
}
