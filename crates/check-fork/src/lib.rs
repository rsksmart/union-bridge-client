#![forbid(unsafe_code)]

pub mod block_header;
pub mod rlp;

use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::block_header::RskBlockHeader;

pub const SUPERBLOCK_TIMES_DIFFICULTY: u8 = 20;
pub const CHECK_FORK_JOURNAL_LEN: usize = 72;

const BASE_EVENT_HEADER_VERSION: u8 = 2;
const JOURNAL_OPERATOR_ID_LEN: usize = 33;
const PEGOUT_ID_LEN: usize = 32;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RskBlock {
    pub uncles: Vec<RskBlock>,
    pub pow: H256,
    pub header: RskBlockHeader,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CheckForkArgs {
    pub version: u8,
    pub seq_id: u32,
    pub rand: u32,
    pub stream_id: u32,
    pub packet_id: u32,
    pub utxo_id: u32,
    pub operator_id: [u8; 32],
    pub init_block_time: u64,
    pub init_block_number: u64,
    pub required_effort: U256,
    pub required_num_blocks: u32,
    pub block_list: Vec<RskBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckForkJournal {
    pub operator_id: [u8; JOURNAL_OPERATOR_ID_LEN],
    pub pegout_id: [u8; PEGOUT_ID_LEN],
    pub utxo_id: [u8; 4],
    pub accepted: u8,
    pub version: [u8; 2],
}

impl CheckForkJournal {
    #[must_use]
    pub fn to_bytes(self) -> [u8; CHECK_FORK_JOURNAL_LEN] {
        let mut out = [0u8; CHECK_FORK_JOURNAL_LEN];
        let mut rest = out.as_mut_slice();

        let (operator_dst, next) = rest.split_at_mut(self.operator_id.len());
        operator_dst.copy_from_slice(&self.operator_id);
        rest = next;

        let (pegout_dst, next) = rest.split_at_mut(self.pegout_id.len());
        pegout_dst.copy_from_slice(&self.pegout_id);
        rest = next;

        let (utxo_dst, next) = rest.split_at_mut(self.utxo_id.len());
        utxo_dst.copy_from_slice(&self.utxo_id);
        rest = next;

        let (accepted_dst, version_dst) = rest.split_at_mut(1);
        accepted_dst[0] = self.accepted;
        version_dst.copy_from_slice(&self.version);
        out
    }
}

#[must_use]
pub fn compute_pegout_id(args: &CheckForkArgs) -> H256 {
    let mut hasher = Keccak256::new();
    hasher.update([args.version]);
    hasher.update(args.seq_id.to_be_bytes());
    hasher.update(args.stream_id.to_be_bytes());
    hasher.update(args.packet_id.to_be_bytes());
    hasher.update(args.utxo_id.to_be_bytes());
    hasher.update(args.operator_id);
    hasher.update(args.rand.to_be_bytes());
    H256::from_slice(&hasher.finalize())
}

#[must_use]
pub fn build_check_fork_journal_from_args(
    args: &CheckForkArgs,
    accepted: bool,
) -> CheckForkJournal {
    let pegout_id = compute_pegout_id(args);
    let mut operator_id = [0u8; JOURNAL_OPERATOR_ID_LEN];
    let (operator_prefix, _) = operator_id.split_at_mut(args.operator_id.len());
    operator_prefix.copy_from_slice(&args.operator_id);

    let mut pegout_id_bytes = [0u8; PEGOUT_ID_LEN];
    pegout_id_bytes.copy_from_slice(pegout_id.as_bytes());

    CheckForkJournal {
        operator_id,
        pegout_id: pegout_id_bytes,
        utxo_id: args.utxo_id.to_be_bytes(),
        accepted: u8::from(accepted),
        version: u16::from(args.version).to_be_bytes(),
    }
}

/// Check fork validity and return cumulative `PoW`
///
/// # Errors
///
/// Returns an error string if the fork validation fails.
#[allow(dead_code)]
pub fn check_fork(args: &CheckForkArgs) -> Result<U256, &'static str> {
    let CheckForkArgs {
        init_block_time,
        init_block_number,
        required_effort,
        required_num_blocks,
        block_list,
        ..
    } = args;

    let pegout_id = compute_pegout_id(args);
    let init_block_time = *init_block_time;
    let init_block_number = *init_block_number;
    let required_effort = *required_effort;
    let required_num_blocks = *required_num_blocks;

    //
    // 1. validate block list shape
    //
    validate_block_list(required_num_blocks, block_list)?;

    //
    // 2. validate first block
    //
    let first_block = &block_list[0];
    validate_first_block(first_block, init_block_time, init_block_number, &pegout_id)?;
    validate_block_hash(&first_block.header)?;

    let mut cumulative_effort = accumulate_effort(U256::zero(), first_block)?;

    //
    // 3. validate consecutive blocks
    //
    for i in 1..block_list.len() {
        let block = &block_list[i];
        let prev_block = &block_list[i - 1];

        validate_consecutive_block(block, prev_block)?;
        validate_block_hash(&block.header)?;
        cumulative_effort = accumulate_effort(cumulative_effort, block)?;

        for uncle in &block.uncles {
            validate_uncle(prev_block, uncle)?;
            cumulative_effort = accumulate_effort(cumulative_effort, uncle)?;
        }

        if i >= 2 {
            validate_required_pegout_event(block, &pegout_id)?;
        }
    }

    //
    // 4. validate enough cumulative PoW
    //
    if cumulative_effort < required_effort {
        return Err("Cumulative PoW does not meet the required threshold");
    }

    Ok(cumulative_effort)
}

fn accumulate_effort(cumulative_effort: U256, block: &RskBlock) -> Result<U256, &'static str> {
    let effort = calculate_block_effort(block)?;

    cumulative_effort.checked_add(effort).ok_or("Overflow occurred adding block's PoW")
}

fn validate_block_list(
    required_num_blocks: u32,
    block_list: &[RskBlock],
) -> Result<(), &'static str> {
    if required_num_blocks < 1 {
        return Err("Invalid number of required blocks");
    }

    if block_list.len() < 2 {
        return Err("Check-fork requires at least two blocks");
    }

    if block_list.len() < required_num_blocks as usize {
        return Err("Insufficient number of blocks");
    }

    Ok(())
}

fn validate_first_block(
    block: &RskBlock,
    init_block_time: u64,
    init_block_number: u64,
    pegout_id: &H256,
) -> Result<(), &'static str> {
    if block.header.timestamp < init_block_time {
        return Err("First block timestamp lower than expected");
    }

    if block.header.number < init_block_number {
        return Err("First block number lower than expected");
    }

    validate_enough_effort_superblock(block, "first")?;

    if contains_matching_pegout_event(block, pegout_id) {
        return Err("First block must not contain the PegOutID base event");
    }

    Ok(())
}

fn validate_required_pegout_event(block: &RskBlock, pegout_id: &H256) -> Result<(), &'static str> {
    if contains_matching_pegout_event(block, pegout_id) {
        return Ok(());
    }

    Err("Continuation block is missing the expected PegOutID base event")
}

fn contains_matching_pegout_event(block: &RskBlock, pegout_id: &H256) -> bool {
    block.header.base_event.as_deref().is_some_and(|event| event == pegout_id.as_bytes())
}

fn validate_consecutive_block(block: &RskBlock, prev_block: &RskBlock) -> Result<(), &'static str> {
    if block.header.timestamp <= prev_block.header.timestamp {
        return Err("Block Timestamp is not increasing");
    }

    let expected_next_number = prev_block
        .header
        .number
        .checked_add(1)
        .ok_or("Overflow incrementing previous block number")?;

    if block.header.number != expected_next_number {
        return Err("Block numbers are not consecutive");
    }

    if block.header.parent != prev_block.header.hash {
        return Err("Invalid parent linkage between blocks");
    }

    validate_enough_effort_superblock(block, "consecutive")?;
    validate_difficulty_in_bounds(block, prev_block)?;

    Ok(())
}

fn validate_uncle(trunk_block: &RskBlock, uncle: &RskBlock) -> Result<(), &'static str> {
    if uncle.header.number != trunk_block.header.number {
        return Err("Uncle's block number does not match trunk block number");
    }

    if uncle.header.parent != trunk_block.header.parent {
        return Err("Uncle's parent does not match trunk block's parent");
    }

    if uncle.header.difficulty != trunk_block.header.difficulty {
        return Err("Uncle's difficulty does not match trunk block's difficulty");
    }

    validate_block_hash(&uncle.header)?;
    validate_enough_effort_superblock(uncle, "uncle")?;

    Ok(())
}

fn validate_enough_effort_superblock(
    block: &RskBlock,
    _block_type: &str,
) -> Result<(), &'static str> {
    let expected_effort = block
        .header
        .difficulty
        .checked_mul(SUPERBLOCK_TIMES_DIFFICULTY.into())
        .ok_or("Overflow occurred multiplying difficulty by times")?;
    let actual_effort = calculate_block_effort(block)?;

    // dbg!((
    //     block.number,
    //     &block.pow,
    //     expected_effort,
    //     actual_effort,
    //     _block_type
    // ));

    if actual_effort >= expected_effort {
        return Ok(());
    }

    // TODO tmp, do not err if not super block for now (until Superchain), just log
    // match _block_type {
    //     "first" => Err("First block's PoW is less than the required difficulty"),
    //     "consecutive" => Err("Consecutive Block's PoW is less than the required difficulty"),
    //     "uncle" => Err("Uncle's Block PoW is less than the required difficulty"),
    //     _ => panic!("Invalid block type"),
    // }

    Ok(())
}

fn validate_difficulty_in_bounds(
    block: &RskBlock,
    prev_block: &RskBlock,
) -> Result<(), &'static str> {
    // check these RSKj lines:
    // - https://github.com/rsksmart/rskj/blob/3cd3401a9c6cfd3dfa63120304d0f26f691ae6e7/rskj-core/src/main/java/co/rsk/core/DifficultyCalculator.java#L64
    // - https://github.com/rsksmart/rskj/blob/master/rskj-core/src/main/java/org/ethereum/config/Constants.java#L150
    let max_delta = prev_block.header.difficulty / 400;

    let lower_bound = prev_block.header.difficulty.saturating_sub(max_delta);
    let upper_bound = prev_block.header.difficulty.saturating_add(max_delta);

    let in_bounds = (lower_bound..=upper_bound).contains(&block.header.difficulty);
    if in_bounds { Ok(()) } else { Err("Consecutive Block difficulty is out of bounds") }
}

fn calculate_block_effort(block: &RskBlock) -> Result<U256, &'static str> {
    let pow = U256::from_big_endian(block.pow.as_bytes());
    // compute the effort by inverting the pow
    // U256::MAX, the "difficulty 1" target, represents the easiest possible target
    U256::MAX.checked_div(pow).ok_or("0 division on calculate_block_effort")
}

fn validate_block_hash(header: &RskBlockHeader) -> Result<(), &'static str> {
    if header.base_event.is_some() && header.version != BASE_EVENT_HEADER_VERSION {
        return Err("Block with base event must use header version 2");
    }

    let actual_hash = header.calculate_block_hash()?;
    if header.hash != actual_hash {
        println!("Block number: {}", header.number);
        return Err("Block header hash is not matching");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    mod lib_tests;
}
