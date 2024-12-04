use check_fork::CheckForkArgs;
use monitor::get_blocks;
use primitive_types::U256;
use std::error::Error;
use zkvm_host::prove_stark;
use zkvm_guest::CHECK_FORK_GUEST_ELF;

// Testing parameters, change for different behaviors
const START_BLOCK_NUMBER: u32 = 6883222;
const NUM_OF_BLOCKS: u16 = 100;
const REQUIRED_EFFORT: u32 = 100;
const INIT_BLOCK_NUMBER: u32 = START_BLOCK_NUMBER - 1;
const INIT_TIMESTAMP: u64 = 1701129600;
const REQUIRED_NUM_BLOCKS: u16 = NUM_OF_BLOCKS;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let blocks = get_blocks(START_BLOCK_NUMBER, NUM_OF_BLOCKS).await?;

    let args = CheckForkArgs {
        utxo_id: "FAKE_UTXO_ID".to_string(),         // tmp
        operator_id: "FAKE_OPERATOR_ID".to_string(), // tmp
        init_block_time: INIT_TIMESTAMP,
        init_block_number: INIT_BLOCK_NUMBER,
        required_effort: U256::from(REQUIRED_EFFORT),
        required_num_blocks: REQUIRED_NUM_BLOCKS,
        block_list: blocks,
    };

    prove_stark(args, CHECK_FORK_GUEST_ELF, "../stark-proof.bin");

    println!("\"Monitor\" done!");

    Ok(())
}
