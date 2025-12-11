use check_fork::block_header::RskBlockHeader;
use common::types::BlockNumber;
use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
use primitive_types::{H256, U256};
use std::ops::Mul;

const DEFAULT_DIFFICULTY: u64 = 500;

pub(crate) fn create_fake_block(number: BlockNumber, effort: U256) -> RskBlock {
    create_fake_block_with_parent(number, effort, None)
}

pub(crate) fn create_fake_block_with_parent(
    number: BlockNumber,
    effort: U256,
    parent_hash: Option<H256>,
) -> RskBlock {
    let block_pow_u = U256::MAX.checked_div(effort).expect("0 division");
    let pow = BlockPow::from(H256::from_slice(&block_pow_u.to_big_endian()));

    let parent_hash = BlockHash::from(parent_hash.unwrap_or_else(H256::zero));
    let timestamp = BlockTimestamp::from(number.value() * 1000);
    let difficulty = BlockDifficulty::from(U256::from(DEFAULT_DIFFICULTY));
    let total_difficulty = difficulty.mul(BlockDifficulty::from(U256::from(1000)));

    let header = RskBlockHeader::new_with(
        number.value(),
        difficulty.value(),
        Some(parent_hash.value()),
        timestamp.value(),
    );
    let block_hash = BlockHash::from(header.calculate_block_hash().expect("hash calculation"));

    RskBlock::new(
        number,
        block_hash,
        parent_hash,
        timestamp,
        difficulty,
        total_difficulty,
        pow,
        vec![],
    )
}

pub(crate) fn create_fake_child_block(parent: &RskBlock, effort: U256) -> RskBlock {
    create_fake_block_with_parent(
        BlockNumber::from(parent.number().value() + 1),
        effort,
        Some(parent.hash().value()),
    )
}
