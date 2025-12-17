use check_fork::block_header::RskBlockHeader;
use common::types::BlockNumber;
use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
use primitive_types::{H256, U256};
use std::ops::Mul;

const FAKE_BLOCK_DIFFICULTY: u64 = 500;

fn calculate_block_hash_with_parent(number: u64, parent: H256) -> H256 {
    let header = RskBlockHeader::new_with(
        number,
        U256::from(FAKE_BLOCK_DIFFICULTY),
        if number == 0 { None } else { Some(parent) },
        number * 1000,
    );
    header.calculate_block_hash().expect("hash calculation")
}

fn calculate_deterministic_block_hash(number: u64) -> H256 {
    let mut current_hash = H256::zero();

    for n in 0..=number {
        current_hash = calculate_block_hash_with_parent(n, current_hash);
    }

    current_hash
}

pub(crate) fn create_fake_block(number: BlockNumber, effort: U256) -> RskBlock {
    create_fake_block_with_parent(number, effort, None)
}

pub(crate) fn create_fake_block_with_parent(
    number: BlockNumber,
    effort: U256,
    prev_block: Option<&RskBlock>,
) -> RskBlock {
    let block_pow_u = U256::MAX.checked_div(effort).expect("0 division");
    let pow = BlockPow::from(H256::from_slice(&block_pow_u.to_big_endian()));

    let parent_hash = match prev_block {
        Some(prev) => prev.hash(),
        None if number.value() == 0 => BlockHash::from(H256::zero()),
        None => BlockHash::from(calculate_deterministic_block_hash(number.value() - 1)),
    };

    let block_hash = BlockHash::from(calculate_block_hash_with_parent(
        number.value(),
        parent_hash.value(),
    ));

    let timestamp = BlockTimestamp::from(number.value() * 1000);
    let difficulty = BlockDifficulty::from(U256::from(FAKE_BLOCK_DIFFICULTY));
    let total_difficulty = difficulty.mul(BlockDifficulty::from(U256::from(1000)));
    let uncles = vec![];

    RskBlock::new(
        number,
        block_hash,
        parent_hash,
        timestamp,
        difficulty,
        total_difficulty,
        pow,
        uncles,
    )
}
