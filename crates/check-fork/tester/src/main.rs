#![forbid(unsafe_code)]

use std::error::Error;
use std::path::PathBuf;

use check_fork::{CheckForkArgs, check_fork, compute_pegout_id};
use check_fork_tester::{
    apply_base_event_fixture, calculate_total_effort, get_blocks, write_check_fork_artifacts,
};
use clap::Parser;
use methods::{CHECK_FORK_GUEST_ID, CHECK_FORK_GUEST_PATH};
use primitive_types::U256;
use zkvm_cli_serde::{serialize_guest_input, serialize_image_id};

const MOCK_CHECK_FORK_PEGOUT_ID_VERSION: u8 = 1;
const MOCK_CHECK_FORK_SEQUENCE_NUMBER: u64 = 1;
const MOCK_CHECK_FORK_STREAM_ID: u64 = 1;
const MOCK_CHECK_FORK_PACKET_NUMBER: u64 = 1;
const MOCK_CHECK_FORK_SLOT_ID: u64 = 4;
const MOCK_CHECK_FORK_OPERATOR_TAKE_PUBKEY_PARITY: u8 = 0x02;
const MOCK_CHECK_FORK_OPERATOR_TAKE_PUBKEY_XONLY: [u8; 32] = [0x11; 32];
const MOCK_CHECK_FORK_BEST_BLOCK_HASH: [u8; 32] = [0x22; 32];

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Operation to perform: [run, elf, sb]
    #[arg(short = 'o', long = "operation", value_parser = ["run", "elf", "sb"], required = true)]
    operation: String,

    //
    // Fetch parameters
    //
    #[arg(long = "output-dir", default_value = "tester-artifacts")]
    output_dir: PathBuf,

    // Start block number
    #[arg(short = 's', long = "fetch-start-block", default_value_t = 6_883_222)]
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
    #[arg(short = 'e', long = "cf-required-effort", value_parser = parse_u256_dec)]
    cf_required_effort: Option<U256>,

    // Initial block number
    #[arg(short = 'i', long = "cf-init-block", default_value_t = 6_883_221)]
    cf_init_block: u64,

    // Initial timestamp
    #[arg(short = 't', long = "cf-init-timestamp", default_value_t = 1_701_129_600)]
    cf_init_timestamp: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli_args = Args::parse();
    println!("CLI {cli_args:?}");

    if cli_args.operation == "sb" {
        let _ = get_blocks(cli_args.fetch_start_block, cli_args.fetch_block_count, true).await?;
        return Ok(());
    }

    let mut blocks =
        get_blocks(cli_args.fetch_start_block, cli_args.fetch_block_count, false).await?;

    let mut check_fork_args = CheckForkArgs {
        pegout_id_version: MOCK_CHECK_FORK_PEGOUT_ID_VERSION,
        sequence_number: U256::from(MOCK_CHECK_FORK_SEQUENCE_NUMBER),
        stream_id: MOCK_CHECK_FORK_STREAM_ID,
        packet_number: MOCK_CHECK_FORK_PACKET_NUMBER,
        slot_id: MOCK_CHECK_FORK_SLOT_ID,
        operator_take_pubkey_parity: MOCK_CHECK_FORK_OPERATOR_TAKE_PUBKEY_PARITY,
        operator_take_pubkey_xonly: MOCK_CHECK_FORK_OPERATOR_TAKE_PUBKEY_XONLY,
        best_block_hash: MOCK_CHECK_FORK_BEST_BLOCK_HASH.into(),
        init_block_time: cli_args.cf_init_timestamp,
        init_block_number: cli_args.cf_init_block,
        required_effort: U256::zero(),
        required_num_blocks: cli_args.cf_required_blocks,
        block_list: Vec::new(),
    };

    let pegout_id = compute_pegout_id(&check_fork_args);
    let base_event = check_fork::build_pegout_base_event_from_id(pegout_id);
    apply_base_event_fixture(&mut blocks, &base_event)?;
    check_fork_args.block_list = blocks;
    check_fork_args.required_effort = match cli_args.cf_required_effort {
        Some(value) => value,
        None => calculate_total_effort(&check_fork_args.block_list)?,
    };

    write_check_fork_artifacts(&cli_args.output_dir, &check_fork_args)?;

    if cli_args.operation == "elf" {
        generate_elf(&check_fork_args, &cli_args.output_dir)?;
    } else if cli_args.operation == "run" {
        match check_fork(&check_fork_args, pegout_id) {
            Ok(effort) => println!("Check Fork returned ACCEPT with cumulative_effort={effort}"),
            Err(error) => println!("Check Fork returned REJECT: {error:?}"),
        }
    }

    Ok(())
}

fn generate_elf(
    check_fork_args: &CheckForkArgs,
    output_dir: &PathBuf,
) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(output_dir)?;
    let check_fork_args_path = output_dir.join("check_fork_args.bin");
    let check_fork_args_path_str = check_fork_args_path.to_str().ok_or("Invalid path")?;

    let start = std::time::Instant::now();
    serialize_guest_input(check_fork_args, check_fork_args_path_str)?;

    let duration = start.elapsed();
    println!(
        "CheckForkArgs serialized to file: {check_fork_args_path_str}. Total time: {duration:?}"
    );
    println!("input={check_fork_args_path_str}");
    println!("elf={CHECK_FORK_GUEST_PATH}");
    println!("image_id={}", serialize_image_id(CHECK_FORK_GUEST_ID));

    Ok(())
}
fn parse_u256_dec(value: &str) -> Result<U256, String> {
    value.parse::<U256>().map_err(|err| err.to_string())
}
