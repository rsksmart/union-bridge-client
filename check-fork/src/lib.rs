use primitive_types::{H256, U256};
use serde::Deserialize;
use serde::Serialize;

// TODO configurable
pub const SUPERBLOCK_TIMES_DIFFICULTY: u8 = 20;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Block {
    pub number: u64,
    pub hash: H256,
    pub parent: H256,
    pub difficulty: U256,
    pub timestamp: u64,
    pub bridge_event: Option<BridgeEvent>,
    pub uncles: Vec<Block>,
    // alternatively we can receive `bitcoinMergedMiningHeader`, but we would need to include bitcoin crate here, etc.
    pub pow: H256,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BridgeEvent {
    pub utxo_id: String,
    pub pegout_id: String,
    pub operator_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CheckForkArgs {
    pub utxo_id: String,
    pub pegout_id: String,
    pub operator_id: String,
    pub init_block_time: u64,
    pub init_block_number: u64,
    pub required_effort: U256,
    pub required_num_blocks: u32,
    pub block_list: Vec<Block>,
}

#[allow(dead_code)]
pub fn check_fork(args: &CheckForkArgs) -> Result<U256, &'static str> {
    let CheckForkArgs {
        utxo_id,
        pegout_id,
        operator_id,
        init_block_time,
        init_block_number,
        required_effort,
        required_num_blocks,
        block_list,
    } = args;

    // extract values directly to avoid dereferencing later
    let init_block_time = *init_block_time;
    let init_block_number = *init_block_number;
    let required_effort = *required_effort;
    let required_num_blocks = *required_num_blocks;

    //
    // 1. validate list size
    //

    validate_block_list(required_num_blocks, block_list)?;

    //
    // 2. validate first block
    //

    let first_block = &block_list[0];
    validate_first_block(
        first_block,
        init_block_time,
        init_block_number,
        utxo_id,
        pegout_id,
        operator_id,
    )?;

    let mut cumulative_effort = accumulate_effort(U256::zero(), first_block)?;

    //
    // 3. validate consecutive blocks
    //
    for i in 1..block_list.len() {
        let block = &block_list[i];
        let prev_block = &block_list[i - 1];

        validate_consecutive_block(block, prev_block)?;
        cumulative_effort = accumulate_effort(cumulative_effort, block)?;

        for uncle in &block.uncles {
            validate_uncle(prev_block, uncle)?;
            cumulative_effort = accumulate_effort(cumulative_effort, uncle)?;
        }
    }

    //
    // 4. validate enough cumulative PoW
    //
    dbg!((block_list.len(), cumulative_effort, required_effort));

    if cumulative_effort < required_effort {
        return Err("Cumulative PoW does not meet the required threshold");
    }

    Ok(cumulative_effort)
}

fn accumulate_effort(cumulative_effort: U256, block: &Block) -> Result<U256, &'static str> {
    let effort = calculate_block_effort(block)?;

    cumulative_effort
        .checked_add(effort)
        .ok_or("Overflow occurred adding block's PoW")
}

fn validate_block_list(required_num_blocks: u32, block_list: &[Block]) -> Result<(), &'static str> {
    if required_num_blocks < 1 {
        return Err("Invalid number of required blocks");
    }

    if (block_list.len() as u32) < required_num_blocks {
        return Err("Insufficient number of blocks");
    }

    Ok(())
}

fn validate_first_block(
    block: &Block,
    init_block_time: u64,
    init_block_number: u64,
    utxo_id: &str,
    pegout_id: &str,
    operator_id: &str,
) -> Result<(), &'static str> {
    if block.timestamp < init_block_time {
        return Err("First block timestamp lower than expected");
    }

    if block.number < init_block_number {
        return Err("First block number lower than expected");
    }

    validate_enough_effort_superblock(block, "first")?;

    validate_bridge_event(&block.bridge_event, utxo_id, pegout_id, operator_id)?;

    Ok(())
}

fn validate_bridge_event(
    bridge_event: &Option<BridgeEvent>,
    utxo_id: &str,
    pegout_id: &str,
    operator_id: &str,
) -> Result<(), &'static str> {
    let bridge_event = bridge_event
        .as_ref()
        .ok_or("First block is missing BridgeEvent")?;

    if bridge_event.pegout_id != pegout_id {
        return Err("BridgeEvent does not match pegoutID");
    }

    if bridge_event.operator_id != operator_id {
        return Err("BridgeEvent does not match operatorID");
    }

    if bridge_event.utxo_id != utxo_id {
        return Err("BridgeEvent does not match utxoID");
    }

    Ok(())
}

fn validate_consecutive_block(block: &Block, prev_block: &Block) -> Result<(), &'static str> {
    if block.bridge_event.is_some() {
        return Err("Only the first block should contain a BridgeEvent");
    }

    // block timestamp should be greater than previous one
    if block.timestamp <= prev_block.timestamp {
        return Err("Block Timestamp is not increasing");
    }

    // blocks should be consecutive
    if block.number != prev_block.number + 1 {
        return Err("Block numbers are not consecutive");
    }

    // previous should be the parent of current one
    if block.parent != prev_block.hash {
        return Err("Invalid parent linkage between blocks");
    }

    validate_enough_effort_superblock(block, "consecutive")?;

    validate_difficulty_in_bounds(block, prev_block)?;

    Ok(())
}

fn validate_uncle(trunk_block: &Block, uncle: &Block) -> Result<(), &'static str> {
    if uncle.number != trunk_block.number {
        return Err("Uncle's block number does not match trunk block number");
    }

    if uncle.parent != trunk_block.parent {
        return Err("Uncle's parent does not match trunk block's parent");
    }

    if uncle.difficulty != trunk_block.difficulty {
        return Err("Uncle's difficulty does not match trunk block's difficulty");
    }

    validate_enough_effort_superblock(uncle, "uncle")?;
    Ok(())
}

fn validate_enough_effort_superblock(block: &Block, _block_type: &str) -> Result<(), &'static str> {
    let expected_effort = block.difficulty * SUPERBLOCK_TIMES_DIFFICULTY;
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

fn validate_difficulty_in_bounds(block: &Block, prev_block: &Block) -> Result<(), &'static str> {
    // check these RSKj lines:
    // - https://github.com/rsksmart/rskj/blob/3cd3401a9c6cfd3dfa63120304d0f26f691ae6e7/rskj-core/src/main/java/co/rsk/core/DifficultyCalculator.java#L64
    // - https://github.com/rsksmart/rskj/blob/master/rskj-core/src/main/java/org/ethereum/config/Constants.java#L150
    let max_delta = prev_block.difficulty / 400;

    let lower_bound = prev_block.difficulty.saturating_sub(max_delta);
    let upper_bound = prev_block.difficulty.saturating_add(max_delta);

    let in_bounds = (lower_bound..=upper_bound).contains(&block.difficulty);
    if in_bounds {
        Ok(())
    } else {
        Err("Consecutive Block difficulty is out of bounds")
    }
}

fn calculate_block_effort(block: &Block) -> Result<U256, &'static str> {
    let pow = U256::from_big_endian(block.pow.as_bytes());
    // compute the effort by inverting the pow
    // U256::MAX, the "difficulty 1" target, represents the easiest possible target
    U256::MAX
        .checked_div(pow)
        .ok_or("0 division on calculate_block_effort")
}

#[cfg(test)]
mod tests {
    mod lib_tests;
}
