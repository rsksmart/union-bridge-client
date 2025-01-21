use check_fork::CheckForkArgs;
use generate_elf_demo::get_blocks;
use primitive_types::U256;
use std::error::Error;
use zkvm_cli_serde::serialize_guest_input;
use zkvm_guest::{CHECK_FORK_GUEST_ID, CHECK_FORK_GUEST_PATH};
// use zkvm_host::prove_stark_no_cli;

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

    // prove_stark_no_cli(&args, CHECK_FORK_GUEST_PATH, "CheckForkArgs.bin");

    let check_fork_args_path = std::env::current_dir()?.join("check_fork_args.bin");
    let check_fork_args_path_str = check_fork_args_path.to_str().ok_or("Invalid path")?;

    let start = std::time::Instant::now();

    serialize_guest_input(&args, check_fork_args_path_str)?;

    let duration = start.elapsed();
    println!(
        "CheckForkArgs serialized to file: {}. Total time: {:?}",
        check_fork_args_path_str, duration
    );

    println!("GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:");
    println!("    - input: {}", check_fork_args_path_str);
    println!("    - elf: {}", CHECK_FORK_GUEST_PATH);
    println!(
        "    - image_id: {}",
        zkvm_cli_serde::serialize_image_id(CHECK_FORK_GUEST_ID)
    );

    Ok(())
}
