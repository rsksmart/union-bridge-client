use std::error::Error;
use std::path::PathBuf;

use check_fork::{CheckForkArgs, check_fork};
use check_fork_tester::{
    apply_a2_base_event, calculate_total_effort, get_blocks, parse_operator_id_hex,
    write_a2_artifacts,
};
use clap::Parser;
use methods::{CHECK_FORK_GUEST_ID, CHECK_FORK_GUEST_PATH};
use primitive_types::U256;
use zkvm_cli_serde::{serialize_guest_input, serialize_image_id};

const DEFAULT_VERSION: u8 = 1;
const DEFAULT_SEQ_ID: u32 = 1;
const DEFAULT_RAND: u32 = 0xA2C0_FFEE;
const DEFAULT_STREAM_ID: u32 = 1;
const DEFAULT_PACKET_ID: u32 = 1;
const DEFAULT_UTXO_ID: u32 = 4;

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

    #[arg(long = "cf-version", default_value_t = DEFAULT_VERSION)]
    cf_version: u8,

    #[arg(long = "cf-seq-id", default_value_t = DEFAULT_SEQ_ID)]
    cf_seq_id: u32,

    #[arg(long = "cf-rand", default_value_t = DEFAULT_RAND, value_parser = parse_u32_hex_or_dec)]
    cf_rand: u32,

    #[arg(long = "cf-stream-id", default_value_t = DEFAULT_STREAM_ID)]
    cf_stream_id: u32,

    #[arg(long = "cf-packet-id", default_value_t = DEFAULT_PACKET_ID)]
    cf_packet_id: u32,

    #[arg(long = "cf-utxo-id", default_value_t = DEFAULT_UTXO_ID)]
    cf_utxo_id: u32,

    #[arg(
        long = "cf-operator-id-hex",
        default_value = "1111111111111111111111111111111111111111111111111111111111111111"
    )]
    cf_operator_id_hex: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli_args = Args::parse();
    println!("CLI {cli_args:?}");

    if cli_args.operation == "sb" {
        let _ = get_blocks(cli_args.fetch_start_block, cli_args.fetch_block_count, true).await?;
        return Ok(());
    }

    let operator_id = parse_operator_id_hex(&cli_args.cf_operator_id_hex)?;
    let mut blocks =
        get_blocks(cli_args.fetch_start_block, cli_args.fetch_block_count, false).await?;

    let mut check_fork_args = CheckForkArgs {
        version: cli_args.cf_version,
        seq_id: cli_args.cf_seq_id,
        rand: cli_args.cf_rand,
        stream_id: cli_args.cf_stream_id,
        packet_id: cli_args.cf_packet_id,
        utxo_id: cli_args.cf_utxo_id,
        operator_id,
        init_block_time: cli_args.cf_init_timestamp,
        init_block_number: cli_args.cf_init_block,
        required_effort: U256::zero(),
        required_num_blocks: cli_args.cf_required_blocks,
        block_list: Vec::new(),
    };

    let pegout_id = check_fork::compute_pegout_id(&check_fork_args);
    apply_a2_base_event(&mut blocks, pegout_id)?;
    check_fork_args.block_list = blocks;
    check_fork_args.required_effort = match cli_args.cf_required_effort {
        Some(value) => value,
        None => calculate_total_effort(&check_fork_args.block_list)?,
    };

    write_a2_artifacts(&cli_args.output_dir, &check_fork_args)?;

    if cli_args.operation == "elf" {
        generate_elf(&check_fork_args, &cli_args.output_dir)?;
    } else if cli_args.operation == "run" {
        match check_fork(&check_fork_args) {
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

fn parse_u32_hex_or_dec(value: &str) -> Result<u32, String> {
    if let Some(stripped) = value.strip_prefix("0x") {
        u32::from_str_radix(stripped, 16).map_err(|err| err.to_string())
    } else {
        value.parse::<u32>().map_err(|err| err.to_string())
    }
}

fn parse_u256_dec(value: &str) -> Result<U256, String> {
    value.parse::<U256>().map_err(|err| err.to_string())
}
