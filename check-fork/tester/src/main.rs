use std::error::Error;

use check_fork::CheckForkArgs;
use check_fork_tester::get_blocks;
use clap::Parser;
use methods::{CHECK_FORK_GUEST_ID, CHECK_FORK_GUEST_PATH};
use primitive_types::U256;
use zkvm_cli_serde::serialize_guest_input;
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Operation to perform: [run, elf, sb]
    #[arg(short = 'o', long = "operation", value_parser = ["run", "elf", "sb"], required = true )]
    operation: String,

    //
    // Fetch parameters
    //

    // Adds a bridge event to the first block
    #[arg(short = 'g', long = "bridge-event", action = clap::ArgAction::Set, default_value_t = true)]
    bridge_event: bool,

    // Start block number
    #[arg(short = 's', long = "fetch-start-block", default_value_t = 6883222)]
    fetch_start_block: u64,

    // Number of blocks to fetch
    #[arg(short = 'b', long = "fetch-block-count", default_value_t = 100)]
    fetch_block_count: u32,

    //
    // Check Fork parameters
    //

    // Required number of blocks
    #[arg(short = 'r', long = "cf-required-blocks", default_value_t = 100)]
    cf_required_blocks: u32,

    // Required effort
    #[arg(short = 'e', long = "cf-required-effort", default_value_t = U256::from(123456789))]
    cf_required_effort: U256,

    // Initial block number
    #[arg(short = 'i', long = "cf-init-block", default_value_t = 6883221)]
    cf_init_block: u64,

    // Initial timestamp
    #[arg(short = 't', long = "cf-init-timestamp", default_value_t = 1701129600)]
    cf_init_timestamp: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli_args = Args::parse();

    println!("CLI {cli_args:?}");

    let log_super_block = cli_args.operation == "sb";

    let blocks = get_blocks(
        cli_args.fetch_start_block,
        cli_args.fetch_block_count,
        log_super_block,
        cli_args.bridge_event,
    )
    .await?;

    let check_fork_args = CheckForkArgs {
        utxo_id: "FAKE_UTXO_ID".to_string(),         // tmp
        pegout_id: "FAKE_PEGOUT_ID".to_string(),     // tmp
        operator_id: "FAKE_OPERATOR_ID".to_string(), // tmp
        init_block_time: cli_args.cf_init_timestamp,
        init_block_number: cli_args.cf_init_block,
        required_effort: cli_args.cf_required_effort,
        required_num_blocks: cli_args.cf_required_blocks,
        block_list: blocks,
    };

    if cli_args.operation == "elf" {
        generate_elf(&check_fork_args)?;
    } else if cli_args.operation == "run" {
        match check_fork::check_fork(&check_fork_args) {
            Ok(_) => println!("Check Fork returned ACCEPT"),
            Err(e) => println!("Check Fork returned REJECT: {e:?}"),
        }
    } // for "sb" operation, no need to do anything, get_blocks() already logs the superblocks

    Ok(())
}

fn generate_elf(check_fork_args: &CheckForkArgs) -> Result<(), Box<dyn Error>> {
    // prove_stark_no_cli(&args, CHECK_FORK_GUEST_PATH, "CheckForkArgs.bin");

    let check_fork_args_path = std::env::current_dir()?.join("check_fork_args.bin");
    let check_fork_args_path_str = check_fork_args_path.to_str().ok_or("Invalid path")?;

    let start = std::time::Instant::now();

    serialize_guest_input(&check_fork_args, check_fork_args_path_str)?;

    let duration = start.elapsed();
    println!(
        "CheckForkArgs serialized to file: {check_fork_args_path_str}. Total time: {duration:?}"
    );

    println!(
        "GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:"
    );
    println!("    - input: {check_fork_args_path_str}");
    println!("    - elf: {CHECK_FORK_GUEST_PATH}");
    println!("    - image_id: {}", zkvm_cli_serde::serialize_image_id(CHECK_FORK_GUEST_ID));
    Ok(())
}
