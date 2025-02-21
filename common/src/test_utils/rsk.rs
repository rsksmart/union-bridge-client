use log::{debug, error, info, warn};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{thread, time::Duration};

use crate::{
    rsk_provider::{MockRskProvider, MockRskSubscription},
    shutdown_flag::ShutdownFlag,
    types::RskBlock,
};
use primitive_types::U256;
use sha2::{Digest, Sha256};

/// Returns a list of default RSK test blocks.
///
/// This function provides a collection of predefined RSK test blocks, which can be used
/// for testing or reference purposes.
///
/// # Example
///
/// ```
/// use common::test_utils::rsk::get_default_rsk_blocks;
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
/// use common::test_utils::rsk::get_first_default_rsk_block;
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
/// use common::test_utils::rsk::get_second_default_rsk_block;
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
/// use common::test_utils::rsk::get_third_default_rsk_block;
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
///
/// # Example
///
/// ```
/// use common::test_utils::rsk::FakeBlockGenerator;
/// use primitive_types::U256;
///
/// let mut generator = FakeBlockGenerator::new();
/// let block = generator.generate_block(5, 0, false);
///
/// assert_eq!(block.number(), 5);
/// ```
///

#[derive(Clone)]
pub struct FakeBlockGenerator {
    /// Base difficulty (e.g. 10^22).
    base_difficulty: U256,
    /// Difficulty increment per block (e.g. 10^16).
    difficulty_increment: U256,
    /// Base timestamp (Unix epoch, in seconds).
    base_timestamp: i64,
    /// Average block time in seconds.
    avg_block_time: i64,
}

impl FakeBlockGenerator {
    /// Creates a new fake block generator with preset base values.
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    ///
    /// let generator = FakeBlockGenerator::new();
    /// ```
    pub fn new() -> Self {
        Self {
            base_difficulty: U256::from_dec_str("10000000000000000000000").unwrap(),
            difficulty_increment: U256::from_dec_str("10000000000000000").unwrap(),
            base_timestamp: 1514980800,
            avg_block_time: 30,
        }
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

    fn generate_hash(&self, height: u64, flavor: &str) -> String {
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
    ///
    /// The block's parent hash is calculated as `generate_hash(height-1)`.
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    ///
    /// let mut generator = FakeBlockGenerator::new();
    /// let block = generator.generate_block(100, 0, true);
    /// // The block height should be set as provided.
    /// assert_eq!(block.number(), 100);
    /// // The parent hash should equal the hash for block 99.
    /// let expected_parent_hash = "0xef2310cd0c172059feb3c382c56acfbc8127222d4d2cc51b78db3019ce1a83f6";
    /// assert_eq!(expected_parent_hash, block.parent());
    /// ```
    pub fn generate_block(&self, height: u64, reorg_block_height: u64, is_reorg: bool) -> RskBlock {
        let parent_hash = if height == 0 {
            "0x0000000000000000000000000000000000000000000000000000000000000000".to_string()
        } else {
            self.generate_hash(
                height - 1,
                if (height > reorg_block_height) && is_reorg {
                    "alt"
                } else {
                    ""
                },
            )
        };
        let block_hash = self.generate_hash(
            height,
            if (height >= reorg_block_height) && is_reorg {
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
}

pub fn set_provider_expect_get_block_by_hash(
    mock: &mut MockRskProvider,
    is_reorg: Arc<AtomicBool>,
    expected_hash_string: String,
    generator: &FakeBlockGenerator,
    block_height: u64,
    block_height_reorg_from: u64,
) {
    let generator_clone = generator.clone();
    mock.expect_get_block_by_hash()
        .with(mockall::predicate::eq(expected_hash_string))
        .returning(move |_hash| {
            let reorg_active = is_reorg.load(Ordering::SeqCst);
            let reorg_from = if reorg_active {
                block_height_reorg_from
            } else {
                0
            };
            Ok(Some(generator_clone.generate_block(
                block_height,
                reorg_from,
                reorg_active,
            )))
        });
}

pub fn set_provider_expect_get_best_block(
    mock: &mut MockRskProvider,
    is_reorg: Arc<AtomicBool>,
    generator: &FakeBlockGenerator,
    block_height: u64,
    block_height_reorg_from: u64,
) {
    let generator_clone = generator.clone();
    mock.expect_get_best_block().returning(move || {
        let reorg_active = is_reorg.load(Ordering::SeqCst);
        let reorg_from = if reorg_active {
            block_height_reorg_from
        } else {
            0
        };
        Ok(generator_clone.generate_block(block_height, reorg_from, reorg_active))
    });
}

pub fn set_provider_expect_get_block_by_number(
    mock: &mut MockRskProvider,
    generator: &FakeBlockGenerator,
    valid_range: std::ops::RangeInclusive<u64>,
) {
    set_provider_expect_get_block_by_number_generic(mock, generator, valid_range, |_| {});
}

pub fn set_provider_expect_get_block_by_number_with_shutdown_at_block(
    mock: &mut MockRskProvider,
    generator: &FakeBlockGenerator,
    shutdown_flag: &ShutdownFlag,
    valid_range: std::ops::RangeInclusive<u64>,
    block_height_at_shutdown: u64,
) {
    let shutdown_flag_clone = shutdown_flag.clone();
    set_provider_expect_get_block_by_number_generic(mock, generator, valid_range, move |height| {
        if height == block_height_at_shutdown {
            shutdown_flag_clone.set(true);
        }
    });
}

pub fn set_provider_expect_get_block_by_number_generic<F>(
    mock: &mut MockRskProvider,
    generator: &FakeBlockGenerator,
    valid_range: std::ops::RangeInclusive<u64>,
    callback: F,
) where
    F: Fn(u64) + Send + Sync + 'static,
{
    let generator_clone = generator.clone();
    mock.expect_get_block_by_number().returning(move |height| {
        if valid_range.contains(&height) {
            callback(height);
            Ok(Some(generator_clone.generate_block(height, 0, false)))
        } else {
            Ok(None)
        }
    });
}

pub fn set_provider_expect_get_block_by_number_with_reorg(
    mock: &mut MockRskProvider,
    is_reorg: Arc<AtomicBool>,
    generator: &FakeBlockGenerator,
    valid_range: std::ops::RangeInclusive<u64>,
    block_height_reorg_happens_at: u64,
    block_height_reorg_from: u64,
) {
    let generator_clone = generator.clone();
    mock.expect_get_block_by_number().returning(move |height| {
        if valid_range.contains(&height) {
            if height == block_height_reorg_happens_at {
                is_reorg.store(true, Ordering::SeqCst);
                info!(
                    "Reorg initiated at block height {} with hash {}",
                    height,
                    generator_clone.generate_hash(height, "alt")
                );
            }
            let reorg_active = is_reorg.load(Ordering::SeqCst);
            let reorg_from = if reorg_active {
                block_height_reorg_from
            } else {
                0
            };
            Ok(Some(generator_clone.generate_block(
                height,
                reorg_from,
                reorg_active,
            )))
        } else {
            Ok(None)
        }
    });
}

pub fn set_provider_expect_subscribe_blocks(
    mock: &mut MockRskProvider,
    is_reorg: Arc<AtomicBool>,
    generator: &FakeBlockGenerator,
    shutdown_flag: &ShutdownFlag,
    block_height_reorg_from: u64,
    block_height_subscription_init: u64,
    block_height_subscription_max: u64,
    delay_between_blocks_subscription: u64,
) {
    let generator_clone = generator.clone();
    let shutdown_flag_clone = shutdown_flag.clone();
    let mut height_subscr_counter = block_height_subscription_init;
    mock.expect_subscribe_blocks().returning({
        move |_shutdown_flag| {
            let mut mock_sub = MockRskSubscription::<RskBlock>::new();
            let generator_clone = generator_clone.clone();
            let is_reorg_clone = is_reorg.clone();
            let shutdown_flag_clone = shutdown_flag_clone.clone();
            mock_sub.expect_next().returning({
                move || {
                    let reorg_active = is_reorg_clone.clone().load(Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(delay_between_blocks_subscription));
                    let block = if reorg_active {
                        generator_clone.generate_block(
                            height_subscr_counter,
                            block_height_reorg_from,
                            reorg_active,
                        )
                    } else {
                        generator_clone.generate_block(height_subscr_counter, 0, reorg_active)
                    };
                    height_subscr_counter += 1;
                    if height_subscr_counter <= block_height_subscription_max {
                        Ok(block)
                    } else {
                        while !shutdown_flag_clone.is_on() {
                            thread::sleep(Duration::from_millis(20));
                        }
                        Ok(block)
                    }
                }
            });
            mock_sub.expect_unsubscribe().returning(|| Ok(()));
            Ok(mock_sub)
        }
    });
}
