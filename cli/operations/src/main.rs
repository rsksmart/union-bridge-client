//! union bridge operator and user operations toolkit
//!
//! organized into three main command groups:
//!
//! ## setup
//! initial configuration commands
//! - `create-rootstock-wallets`: creates rootstock keystores for local multi-client deployments
//!
//! ## operator
//! commands for managing committee members and funding
//! - `fund`: collects bitcoin addresses and funds rootstock wallets for operators
//! - `apply-stream`: applies operator to a stream for committee setup
//!   - for local: applies all 4 operators automatically
//!   - for alphanet/testnet: requires `--operator-id` (1-4) and `--role` (prover/verifier)
//!
//! ## user
//! pegin/pegout operations
//! - `pegin`: requests a pegin address and prints bitcoin-wallet instructions
//! - `pegout`: requests a pegout (withdrawal from Rootstock to Bitcoin)
//!
//! quick examples:
//! ```bash
//! cargo run -- setup create-rootstock-wallets
//! cargo run -- operator fund --env local-docker
//! cargo run -- operator apply-stream -s 1 --env alphanet -o 1 -r prover
//! cargo run -- user pegin -a 0x1234...cdef -s 2_000_000 -p 7
//! cargo run -- user pegout -a 1000000
//! ```

mod bitcoin_wallet;
mod committee;
mod constants;
mod environments;
mod pegin;
mod pegout;
mod rsk_wallet;
mod utils;

use crate::committee::CommitteeRole;
use crate::constants::OPERATOR_IDS;
use crate::environments::Environment;
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser, Clone)]
#[command(
    name = "operations",
    about = "Union Bridge Operator and User Operations"
)]
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
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local)]
        env: Environment,
    },
    /// Apply operator to a stream for committee setup
    #[command(name = "apply-stream")]
    ApplyToStream {
        /// Stream identifier to configure
        #[arg(short = 's', long = "stream-id", value_name = "STREAM_ID")]
        stream_id: u64,

        /// Target environment (local, alphanet, testnet)
        #[arg(short = 'e', long = "env", value_enum, default_value_t = Environment::Local)]
        env: Environment,

        /// Operator ID (1-4) when applying on alphanet or testnet
        #[arg(short = 'o', long = "operator-id", value_name = "OPERATOR_ID")]
        operator_id: Option<u8>,

        /// Operator role when applying on alphanet or testnet
        #[arg(short = 'r', long = "role", value_enum)]
        role: Option<CommitteeRole>,
    },
}

#[derive(Debug, Subcommand, Clone)]
enum UserCommands {
    /// Request a pegin address and print bitcoin-wallet CLI instructions
    Pegin {
        /// Environment to target (local, local-docker, alphanet, testnet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local)]
        env: Environment,

        /// Rootstock deposit address
        #[arg(short = 'a', long = "rsk-address", value_name = "RSK_ADDRESS")]
        rsk_address: String,

        /// Amount (in satoshis) to stream into the pegin transaction
        #[arg(
            short = 's',
            long = "stream-amount",
            value_name = "STREAM_AMOUNT",
            default_value_t = 1_000_000u64
        )]
        stream_amount: u64,

        /// Packet number used when creating the pegin transaction
        #[arg(
            short = 'p',
            long = "packet-number",
            value_name = "PACKET_NUMBER",
            default_value_t = 0u64
        )]
        packet_number: u64,
    },
    /// Request a pegout (withdraw from Rootstock to Bitcoin)
    Pegout {
        /// Environment to target (local, local-docker, alphanet, testnet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local)]
        env: Environment,

        /// Amount in satoshis to withdraw
        #[arg(short = 'a', long = "amount", value_name = "AMOUNT_SATS")]
        amount_sats: u64,
    },
}

fn validate_1_4(value: u8, name: &str) -> Result<()> {
    if !(1..=4).contains(&value) {
        anyhow::bail!("{} must be between 1 and 4", name);
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
                    OPERATOR_IDS.len() as u8,
                    base_storage_path.as_deref(),
                )?;
            }
        },
        Commands::Operator { command } => match command {
            OperatorCommands::Fund { env } => {
                println!("=== Funding Bitcoin addresses ===");
                bitcoin_wallet::handle_bitcoin_funding(env).await?;
                println!("\n=== Funding Rootstock wallets ===");
                rsk_wallet::handle_operator_funding(env).await?;
            }
            OperatorCommands::ApplyToStream {
                stream_id,
                env,
                operator_id,
                role,
            } => {
                committee::run_committee_setup(stream_id, env, operator_id, role).await?;
            }
        },
        Commands::User { command } => match command {
            UserCommands::Pegin {
                env,
                rsk_address,
                stream_amount,
                packet_number,
            } => {
                pegin::create_pegin_tx(env, rsk_address, stream_amount, packet_number).await?;
            }
            UserCommands::Pegout { env, amount_sats } => {
                pegout::request_pegout(env, amount_sats).await?;
            }
        },
    }

    Ok(())
}
