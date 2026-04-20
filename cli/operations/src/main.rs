//! union bridge operator and user operations toolkit
//!
//! provides commands for interacting with the union bridge protocol.
//! local bootstrap is handled by `cli-setup-operators.sh`.
//!
//! ## operator
//! commands for operator wallet management and committee registration
//! - `fund`: displays operator bitcoin addresses and funds rootstock wallets
//!   - prints bitcoin addresses that need to be funded manually in bitcoin-wallet cli
//!   - with `--execute` flag: runs the wallet commands programmatically via cli-bitcoin-wallet.sh
//!   - automatically funds rootstock addresses via anvil in local mode
//! - `apply-stream`: registers operator(s) to a stream for committee participation
//!   - local: applies all 4 operators automatically
//!   - remote profiles: require `--operator-id` (1-10) and `--role` (prover/verifier)
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
//! fund operator wallets (local):
//! ```bash
//! cargo run -- operator fund --env local
//! # copy displayed bitcoin addresses and fund them in bitcoin-wallet cli
//! # or use --execute to run the wallet commands automatically:
//! cargo run -- operator fund --env local --execute
//! # optionally override the derived funding amount in satoshis:
//! cargo run -- operator fund --env local --execute --fund-amount 65000000
//! ```
//!
//! apply operators to stream (local - all 4 operators):
//! ```bash
//! cargo run -- operator apply-stream -s 0 --env local
//! ```
//!
//! apply operator to stream (remote profile - single operator):
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
use op_funding::derive_stream_funding_profile;

use crate::committee::CommitteeRole;
use crate::constants::{operator_and_prover_counts, COMMITTEE_PACKET_SIZE};
use crate::environments::Environment;

#[derive(Debug, Parser, Clone)]
#[command(name = "operations", about = "Union Bridge Operator and User Operations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand, Clone)]
enum Commands {
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
enum OperatorCommands {
    /// Fund operators
    Fund {
        /// Environment to target (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(long = "env", short = 'e', default_value = "local", env = "UC_ENV")]
        env: Environment,

        /// Stream identifier used to derive BitVMX funding and UTXO values
        #[arg(short = 's', long = "stream", value_name = "STREAM_ID", default_value_t = 0)]
        stream_id: u64,

        /// StreamManager contract address. Required for remote environments when
        /// funding operator RSK balances.
        #[arg(long = "stream-manager-address", value_name = "ADDRESS")]
        stream_manager_address: Option<String>,

        /// Execute the wallet commands programmatically instead of just printing them
        #[arg(long = "execute", default_value_t = false)]
        execute: bool,

        /// Override the derived Bitcoin funding amount in satoshis
        #[arg(
            long = "fund-amount",
            value_name = "SATOSHIS",
            value_parser = clap::value_parser!(u64).range(1..)
        )]
        fund_amount: Option<u64>,
    },
    /// Whitelist member addresses on the CommitteeRegistry contract
    Whitelist {
        /// Environment to target (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(long = "env", short = 'e', default_value = "local")]
        env: Environment,

        /// CommitteeRegistry contract address
        #[arg(long = "contract-address", value_name = "ADDRESS")]
        contract_address: String,

        /// Sender address to use with `cast send --from ... --unlocked` in local/docker.
        /// Defaults to the local anvil account if not provided.
        #[arg(long = "from", value_name = "ADDRESS", conflicts_with = "private_key")]
        from_address: Option<String>,

        /// Private key to use in remote profile mode.
        /// If omitted in remote environments, it is prompted interactively.
        #[arg(long = "private-key", value_name = "HEX_KEY", conflicts_with = "from_address")]
        private_key: Option<String>,
    },
    /// Print the per-operator Bitcoin funding amount (sats) for the given stream.
    /// Emits a single integer to stdout so it can be consumed by scripts.
    #[command(name = "funding-amount")]
    FundingAmount {
        /// Environment to target (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(long = "env", short = 'e', default_value = "local", env = "UC_ENV")]
        env: Environment,

        /// Stream identifier used to derive BitVMX funding
        #[arg(short = 's', long = "stream", value_name = "STREAM_ID", default_value_t = 0)]
        stream_id: u64,
    },
    /// Apply operator to a stream for committee setup
    #[command(name = "apply-stream")]
    ApplyToStream {
        /// Stream identifier to configure
        #[arg(short = 's', long = "stream", value_name = "STREAM_ID")]
        stream_id: u64,

        /// Target environment (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(short = 'e', long = "env", default_value = "local", env = "UC_ENV")]
        env: Environment,

        /// Operator ID (1-10) when applying in remote profile mode
        #[arg(
            short = 'o',
            long = "operator-id",
            value_name = "OPERATOR_ID",
            env = "UC_OPERATOR_ID"
        )]
        operator_id: Option<u8>,

        /// Operator role when applying in remote profile mode
        #[arg(short = 'r', long = "role", value_enum, env = "UC_OPERATOR_ROLE")]
        role: Option<CommitteeRole>,
    },
}

#[derive(Debug, Subcommand, Clone)]
enum UserCommands {
    /// Display user addresses and funding instructions
    Fund {
        /// Environment to target (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(long = "env", short = 'e', default_value = "local", env = "UC_ENV")]
        env: Environment,
    },
    /// Request a pegin address and print bitcoin-wallet CLI instructions
    Pegin {
        /// Environment to target (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(long = "env", short = 'e', default_value = "local", env = "UC_ENV")]
        env: Environment,

        /// Rootstock deposit address
        #[arg(short = 'a', long = "rsk-address", value_name = "RSK_ADDRESS")]
        rsk_address: String,

        /// Value in satoshis
        #[arg(short = 'v', long = "value", value_name = "VALUE")]
        value: u64,

        /// Bitcoin public key for reimbursement (32-byte x-only pubkey with 0x prefix)
        #[arg(short = 'k', long = "btc-pub-key", value_name = "BTC_PUB_KEY")]
        btc_pub_key: String,

        /// Execute the wallet command programmatically instead of just printing it
        #[arg(long = "execute", default_value_t = false)]
        execute: bool,
    },
    /// Request a pegout (withdraw from Rootstock to Bitcoin)
    Pegout {
        /// Environment to target (`local`, `docker`, or a remote profile name such as `alphanet`)
        #[arg(long = "env", short = 'e', default_value = "local", env = "UC_ENV")]
        env: Environment,

        /// Value in satoshis
        #[arg(short = 'v', long = "value", value_name = "VALUE")]
        value: u64,

        /// User public key for pegout (33-byte compressed pubkey with 0x prefix)
        #[arg(short = 'k', long = "usr-pub-key", value_name = "USR_PUB_KEY")]
        usr_pub_key: String,
    },
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
        Commands::Operator { command } => match command {
            OperatorCommands::Fund {
                env,
                stream_id,
                stream_manager_address,
                execute,
                fund_amount,
            } => {
                println!("\n=== Funding Rootstock wallets ===");
                rsk_wallet::handle_operator_funding(
                    env.clone(),
                    stream_id,
                    stream_manager_address.as_deref(),
                )
                .await?;
                println!("=== Funding Bitcoin addresses ===");
                bitcoin_wallet::handle_bitcoin_funding(env, stream_id, execute, fund_amount)
                    .await?;
            }
            OperatorCommands::Whitelist { env, contract_address, from_address, private_key } => {
                rsk_wallet::handle_whitelist(
                    env,
                    &contract_address,
                    from_address.as_deref(),
                    private_key.as_deref(),
                )?;
            }
            OperatorCommands::FundingAmount { env, stream_id } => {
                let is_regtest = matches!(env, Environment::Local | Environment::Docker);
                let (operator_count, prover_count) = operator_and_prover_counts();
                let profile = derive_stream_funding_profile(
                    stream_id,
                    is_regtest,
                    COMMITTEE_PACKET_SIZE,
                    operator_count,
                    prover_count,
                )
                .ok_or_else(|| anyhow::anyhow!("invalid stream id {} (expected 0-4)", stream_id))?;
                println!("{}", profile.operator_fund_amount);
            }
            OperatorCommands::ApplyToStream { stream_id, env, operator_id, role } => {
                committee::run_committee_setup(stream_id, env, operator_id, role).await?;
            }
        },
        Commands::User { command } => match command {
            UserCommands::Fund { env } => {
                rsk_wallet::handle_user_funding(env)?;
            }
            UserCommands::Pegin { env, rsk_address, value, btc_pub_key, execute } => {
                pegin::create_pegin_tx(env, rsk_address, value, btc_pub_key, execute).await?;
            }
            UserCommands::Pegout { env, value, usr_pub_key } => {
                pegout::request_pegout(env, value, usr_pub_key).await?;
            }
        },
    }

    Ok(())
}
