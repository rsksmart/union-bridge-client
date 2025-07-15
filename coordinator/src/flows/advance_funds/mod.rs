pub mod advance_funds_flow;
mod check_fork_accumulator;

#[cfg(test)]
pub(crate) mod tests {
    use common::types::BlockNumber;
    use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
    use primitive_types::{H256, U256};
    use std::ops::Mul;

    pub(crate) fn create_fake_block(number: BlockNumber, effort: U256) -> RskBlock {
        let block_pow_u = U256::MAX.checked_div(effort).expect("0 division");
        let pow = BlockPow::from(H256::from_slice(&block_pow_u.to_big_endian()));

        let block_number = number;
        let block_hash = BlockHash::from(H256::from_low_u64_be(number.value()));
        let parent_hash = BlockHash::from(H256::from_low_u64_be(number.value() - 1));
        let timestamp = BlockTimestamp::from(number.value() * 1000);
        let difficulty = BlockDifficulty::from(U256::from(500));
        let total_difficulty = difficulty.mul(BlockDifficulty::from(U256::from(1000)));
        let uncles = vec![];

        RskBlock::new(
            block_number,
            block_hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            pow,
            uncles,
        )
    }
}
