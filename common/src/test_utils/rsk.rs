use crate::types::RskBlock;
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

/// A generator for fake RSK blocks that computes dynamic values (difficulty, timestamp,
/// and total difficulty) based on the block number. This generator is stateless regarding the
/// parent hash; it calculates the parent hash as `generate_hash(num-1)`.
///
/// # Example
///
/// ```
/// use common::test_utils::rsk::FakeBlockGenerator;
/// use primitive_types::U256;
///
/// let mut generator = FakeBlockGenerator::new();
/// let block1 = generator.generate_block(1);
/// let block2 = generator.generate_block(2);
/// let block3 = generator.generate_block(3);
///
/// assert_eq!(block1.number(), 1);
/// assert_eq!(block2.number(), 2);
/// assert_eq!(block3.number(), 3);
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

    /// Computes the difficulty for a given block number.
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    /// use primitive_types::U256;
    ///
    /// let generator = FakeBlockGenerator::new();
    /// let diff = generator.difficulty(100);
    /// // diff should be greater than the base difficulty
    /// assert!(diff > U256::from_dec_str("10000000000000000000000").unwrap());
    /// ```
    pub fn difficulty(&self, num: u64) -> U256 {
        self.base_difficulty + U256::from(num) * self.difficulty_increment
    }

    /// Computes the timestamp for a given block number.
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    ///
    /// let generator = FakeBlockGenerator::new();
    /// let ts = generator.timestamp(10);
    /// // Should increase by approximately 30 seconds per block.
    /// assert!(ts > 1514980800);
    /// ```
    pub fn timestamp(&self, num: u64) -> i64 {
        self.base_timestamp + (num as i64) * self.avg_block_time
    }

    /// Computes the total difficulty up to and including the given block number.
    ///
    /// Uses the formula: T(n) = n * base_difficulty + difficulty_increment * (n*(n+1)/2)
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    /// use primitive_types::U256;
    ///
    /// let generator = FakeBlockGenerator::new();
    /// let tot_diff = generator.total_difficulty(10);
    /// // The total difficulty should be greater than the base difficulty.
    /// assert!(tot_diff > U256::from_dec_str("10000000000000000000000").unwrap());
    /// ```
    pub fn total_difficulty(&self, num: u64) -> U256 {
        let n = U256::from(num);
        let sum_n = n * (n + U256::one()) / U256::from(2u32);
        n * self.base_difficulty + self.difficulty_increment * sum_n
    }

    /// Generates a realistic fake hash based on the block number.
    ///
    /// If `num` is 0, returns the genesis hash.
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    ///
    /// let generator = FakeBlockGenerator::new();
    /// let hash0 = generator.generate_hash(0);
    /// assert_eq!(hash0, "0x0000000000000000000000000000000000000000000000000000000000000000");
    /// ```
    pub fn generate_hash(&self, num: u64) -> String {
        if num == 0 {
            return "0x0000000000000000000000000000000000000000000000000000000000000000"
                .to_string();
        }
        let mut hasher = Sha256::new();
        hasher.update(num.to_le_bytes());
        let result = hasher.finalize();
        format!("0x{:064x}", result)
    }

    /// Generates a fake RSK block for the given block number.
    ///
    /// The block's parent hash is calculated as `generate_hash(num-1)`.
    ///
    /// # Example
    ///
    /// ```
    /// use common::test_utils::rsk::FakeBlockGenerator;
    ///
    /// let mut generator = FakeBlockGenerator::new();
    /// let block = generator.generate_block(100);
    /// // The block number should be set as provided.
    /// assert_eq!(block.number(), 100);
    /// // The parent hash should equal the hash for block 99.
    /// let expected_parent = generator.generate_hash(99);
    /// assert_eq!(block.parent(), expected_parent);
    /// ```
    pub fn generate_block(&self, num: u64) -> RskBlock {
        let parent_hash = self.generate_hash(num - 1);
        let new_hash = self.generate_hash(num);
        let diff = self.difficulty(num);
        let tot_diff = self.total_difficulty(num);
        let ts = self.timestamp(num);
        RskBlock::new(
            num,
            new_hash,
            parent_hash,
            diff,
            ts as u64,
            "pow_string".to_string(),
            tot_diff,
        )
    }
}
