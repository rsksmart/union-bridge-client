//! union bridge operator and user operations toolkit
//!
//! provides commands for setting up and interacting with the union bridge protocol.
//! organized into three main command groups:
//!
//! ## setup
//! initial configuration commands for local development
//! - `create-rootstock-wallets`: creates rootstock keystores for local multi-client deployments
//!
//! ## operator
//! commands for operator wallet management and committee registration
//! - `fund`: displays operator bitcoin addresses and funds rootstock wallets
//!   - prints bitcoin addresses that need to be funded manually in bitcoin-wallet cli
//!   - with `--execute` flag: runs the wallet commands programmatically via cli-bitcoin-wallet.sh
//!   - automatically funds rootstock addresses via anvil (local) or faucet (testnet/alphanet)
//! - `apply-stream`: registers operator(s) to a stream for committee participation
//!   - local: applies all 4 operators automatically
//!   - alphanet/testnet: requires `--operator-id` (1-10) and `--role` (prover/verifier)
//!
//! ## user
//! funding, pegin and pegout transaction commands
//! - `fund`: displays user addresses and funding instructions
//!   - extracts user RSK addresses from user-api logs
//!   - prints cast commands to fund RSK addresses
//!   - prints bitcoin-wallet instructions for funding bitcoin
//! - `pegin`: initiates a bitcoin → rootstock transfer
//!   - prints bitcoin-wallet cli command to execute the pegin transaction
//!   - with `--execute` flag: runs the wallet command programmatically via cli-bitcoin-wallet.sh
//!   - requires: rootstock address, value in satoshis
//!   - packet number is auto-calculated from StreamManager contract (can be overridden with -p)
//! - `pegout`: initiates a rootstock → bitcoin withdrawal
//!   - executes the pegout request on the rootstock side
//!   - requires: value in satoshis
//!
//! ## examples
//!
//! setup local environment:
//! ```bash
//! cargo run -- setup create-rootstock-wallets
//! ```
//!
//! fund operator wallets (local):
//! ```bash
//! cargo run -- operator fund --env local
//! # copy displayed bitcoin addresses and fund them in bitcoin-wallet cli
//! # or use --execute to run the wallet commands automatically:
//! cargo run -- operator fund --env local --execute
//! # optionally specify a custom funding amount in satoshis (default: 32002000):
//! cargo run -- operator fund --env local --execute --fund-amount 65000000
//! ```
//!
//! apply operators to stream (local - all 4 operators):
//! ```bash
//! cargo run -- operator apply-stream -s 0 --env local
//! ```
//!
//! apply operator to stream (alphanet - single operator):
//! ```bash
//! cargo run -- operator apply-stream -s 1 --env alphanet -o 1 -r prover
//! ```
//!
//! fund user wallets (display addresses and instructions):
//! ```bash
//! cargo run -- user fund --env local
//! ```
//!
//! request pegin (bitcoin → rootstock):
//! ```bash
//! cargo run -- user pegin -a 0x1234...cdef -v 100000 -p 0 -k 0x<32-byte-xonly-pubkey> --env local
//! # execute the printed bitcoin-wallet cli command
//! # or use --execute to run the wallet command automatically:
//! cargo run -- user pegin -a 0x1234...cdef -v 100000 -p 0 -k 0x<32-byte-xonly-pubkey> --env local --execute
//! ```
//!
//! request pegout (rootstock → bitcoin):
//! ```bash
//! cargo run -- user pegout -v 100000 -k 0x<33-byte-compressed-pubkey> --env local
//! ```

mod bitcoin_wallet;
mod committee;
mod constants;
mod environments;
mod pegin;
mod pegout;
mod rsk_wallet;
mod utils;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::committee::CommitteeRole;
use crate::constants::operator_ids;
use crate::environments::Environment;

#[derive(Debug, Parser, Clone)]
#[command(name = "operations", about = "Union Bridge Operator and User Operations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand, Clone)]
enum Commands {
    /// Setup commands for initial configuration
    Setup {
        #[command(subcommand)]
        command: SetupCommands,
    },
    /// Operator commands for managing committee members
    Operator {
        #[command(subcommand)]
        command: OperatorCommands,
    },
    /// User commands
    User {
        #[command(subcommand)]
        command: UserCommands,
    },
}

#[derive(Debug, Subcommand, Clone)]
enum SetupCommands {
    /// Create Rootstock wallets for local multi-client deployment
    #[command(name = "create-rootstock-wallets")]
    CreateRootstockWallets,
}

#[derive(Debug, Subcommand, Clone)]
enum OperatorCommands {
    /// Fund operators
    Fund {
        /// Environment to target (local, local-docker, alphanet, testnet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local, env = "UC_ENV")]
        env: Environment,

        /// Execute the wallet commands programmatically instead of just printing them
        #[arg(long = "execute", default_value_t = false)]
        execute: bool,

        /// Bitcoin funding amount in satoshis
        #[arg(
            long = "fund-amount",
            value_name = "SATOSHIS",
            default_value_t = 32_002_000u64,
            value_parser = clap::value_parser!(u64).range(1..)
        )]
        fund_amount: u64,
    },
    /// Apply operator to a stream for committee setup
    #[command(name = "apply-stream")]
    ApplyToStream {
        /// Stream identifier to configure
        #[arg(short = 's', long = "stream-id", value_name = "STREAM_ID")]
        stream_id: u64,

        /// Target environment (local, alphanet, testnet)
        #[arg(short = 'e', long = "env", value_enum, default_value_t = Environment::Local, env = "UC_ENV")]
        env: Environment,

        /// Operator ID (1-10) when applying on alphanet or testnet
        #[arg(
            short = 'o',
            long = "operator-id",
            value_name = "OPERATOR_ID",
            env = "UC_OPERATOR_ID"
        )]
        operator_id: Option<u8>,

        /// Operator role when applying on alphanet or testnet
        #[arg(short = 'r', long = "role", value_enum, env = "UC_OPERATOR_ROLE")]
        role: Option<CommitteeRole>,
    },
}

#[derive(Debug, Subcommand, Clone)]
enum UserCommands {
    /// Display user addresses and funding instructions
    Fund {
        /// Environment to target (local, local-docker, alphanet, testnet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local, env = "UC_ENV")]
        env: Environment,
    },
    /// Request a pegin address and print bitcoin-wallet CLI instructions
    Pegin {
        /// Environment to target (local, local-docker, alphanet, testnet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local, env = "UC_ENV")]
        env: Environment,

        /// Rootstock deposit address
        #[arg(short = 'a', long = "rsk-address", value_name = "RSK_ADDRESS")]
        rsk_address: String,

        /// Value in satoshis
        #[arg(short = 'v', long = "value", value_name = "VALUE")]
        value: u64,

        /// Stream ID to query for the packet number (default: 0)
        #[arg(short = 's', long = "stream-id", value_name = "STREAM_ID", default_value_t = 0u64)]
        stream_id: u64,

        /// Packet number override. If omitted, auto-calculated from the StreamManager contract.
        #[arg(short = 'p', long = "packet-number", value_name = "PACKET_NUMBER")]
        packet_number: Option<u64>,

        /// StreamManager contract address override. If omitted, uses the default for the environment.
        #[arg(long = "stream-manager-address", value_name = "ADDRESS")]
        stream_manager_address: Option<String>,

        /// Bitcoin public key for reimbursement (32-byte x-only pubkey with 0x prefix)
        #[arg(short = 'k', long = "btc-pub-key", value_name = "BTC_PUB_KEY")]
        btc_pub_key: String,

        /// Execute the wallet command programmatically instead of just printing it
        #[arg(long = "execute", default_value_t = false)]
        execute: bool,
    },
    /// Request a pegout (withdraw from Rootstock to Bitcoin)
    Pegout {
        /// Environment to target (local, local-docker, alphanet, testnet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local, env = "UC_ENV")]
        env: Environment,

        /// Value in satoshis
        #[arg(short = 'v', long = "value", value_name = "VALUE")]
        value: u64,

        /// User public key for pegout (33-byte compressed pubkey with 0x prefix)
        #[arg(short = 'k', long = "usr-pub-key", value_name = "USR_PUB_KEY")]
        usr_pub_key: String,
    },
}

fn validate_1_4(value: u8, name: &str) -> Result<()> {
    if !(1..=4).contains(&value) {
        anyhow::bail!("{} must be between 1 and 4", name);
    }
    Ok(())
}

fn validate_1_10(value: u8, name: &str) -> Result<()> {
    if !(1..=10).contains(&value) {
        anyhow::bail!("{} must be between 1 and 10", name);
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup { command } => match command {
            SetupCommands::CreateRootstockWallets => {
                let base_storage_path = std::env::var("BASE_STORAGE_PATH").ok();
                rsk_wallet::handle_wallet_creation(
                    operator_ids().len() as u8,
                    base_storage_path.as_deref(),
                )?;
            }
        },
        Commands::Operator { command } => match command {
            OperatorCommands::Fund { env, execute, fund_amount } => {
                println!("\n=== Funding Rootstock wallets ===");
                rsk_wallet::handle_operator_funding(env).await?;
                println!("=== Funding Bitcoin addresses ===");
                bitcoin_wallet::handle_bitcoin_funding(env, execute, fund_amount).await?;
            }
            OperatorCommands::ApplyToStream { stream_id, env, operator_id, role } => {
                committee::run_committee_setup(stream_id, env, operator_id, role).await?;
            }
        },
        Commands::User { command } => match command {
            UserCommands::Fund { env } => {
                rsk_wallet::handle_user_funding(env)?;
            }
            UserCommands::Pegin {
                env,
                rsk_address,
                value,
                stream_id,
                packet_number,
                stream_manager_address,
                btc_pub_key,
                execute,
            } => {
                pegin::create_pegin_tx(
                    env,
                    rsk_address,
                    value,
                    stream_id,
                    packet_number,
                    stream_manager_address,
                    btc_pub_key,
                    execute,
                )
                .await?;
            }
            UserCommands::Pegout { env, value, usr_pub_key } => {
                pegout::request_pegout(env, value, usr_pub_key).await?;
            }
        },
    }

    Ok(())
}
