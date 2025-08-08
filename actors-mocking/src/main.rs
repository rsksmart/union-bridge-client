use actors_mocking::bitvmx;
use actors_mocking::events;
use alloy_node_bindings::Anvil;
use alloy_provider::{ProviderBuilder, network::EthereumWallet};
use alloy_signer_local::LocalSigner;
use anyhow::{Context, Result};
use bitcoin::{Transaction, Txid};
use clap::{CommandFactory, Parser};
use common::msg_broker::bitvmx_types::PegOutAccepted;
use common::msg_broker::broker::BitVmxBrokerServer;
use hex::FromHex;
use musig2::{PartialSignature, PubNonce};
use std::path::PathBuf;
use std::str::FromStr;
use std::{
    io::Write,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::io::{self, AsyncBufReadExt, BufReader};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "Mock Union Bridge Contracts and BitVMX Client",
    about = "This CLI allows us to emit mocked Union Bridge contract events (invoking mocked UB Contracts) and to emulate BitVMX client requests to the Union Client",
    arg_required_else_help = true
)]
enum Menu {
    Exit,

    //
    // Emit mocked Union Bridge contract events (by invoking mocked UB Contracts)
    //
    #[command(visible_alias = "raf")]
    InvokeRequestAdvanceFunds,

    #[command(visible_alias = "kaf")]
    InvokeAdvanceFunds {
        #[arg(help = "The ID of the pegout")]
        pegout_id: String,
    },

    //
    // Mock received BitVMX events
    //
    #[command(name = "pegin-found", visible_alias = "pf")]
    PeginTransactionFound {
        #[arg(long, help = "Path to BTC transaction JSON file")]
        btc_tx_file: PathBuf,
    },
    #[command(name = "pegin-requested", visible_alias = "pr")]
    PeginRequested {
        #[arg(help = "Bitcoin block hash (hex)")]
        block_hash: String,
        #[arg(long, help = "Path to BTC transaction JSON file")]
        btc_tx_file: PathBuf,
        #[arg(help = "Merkle branch path (hex)")]
        merkle_branch_path: String,
        #[arg(help = "Merkle branch hashes (comma-separated hex values)")]
        merkle_branch_hashes: String,
    },
    #[command(name = "pegin-accepted", visible_alias = "pa")]
    PeginAccepted {
        #[arg(help = "Bitcoin block hash (hex)")]
        block_hash: String,
        #[arg(long, help = "Path to BTC transaction JSON file")]
        btc_tx_file: PathBuf,
        #[arg(help = "Merkle branch path (hex)")]
        merkle_branch_path: String,
        #[arg(help = "Merkle branch hashes (comma-separated hex values)")]
        merkle_branch_hashes: String,
    },
    #[command(name = "pegout-requested", visible_alias = "por")]
    PegoutRequested {
        #[arg(help = "Amount in wei")]
        amount_in_wei: u64,
        #[arg(help = "User public key (hex)")]
        usr_pub_key: String,
    },
    #[command(name = "register-pegout", visible_alias = "rpo")]
    RegisterPegout {
        #[arg(help = "Bitcoin block hash (hex)")]
        block_hash: String,
        #[arg(long, help = "Path to BTC transaction JSON file")]
        btc_tx_file: PathBuf,
        #[arg(help = "Merkle branch path (hex)")]
        merkle_branch_path: String,
        #[arg(help = "Merkle branch hashes (comma-separated hex values)")]
        merkle_branch_hashes: String,
    },
    #[command(name = "send-signature", visible_alias = "ss")]
    PegoutAccepted {
        #[arg(help = "Flow ID (UUID)")]
        flow_id: String,
        #[arg(help = "Committee name/UUID")]
        committee_id: String,
        #[arg(help = "User take txid (hex)")]
        user_take_txid: String,
        #[arg(help = "User take sighash (hex)")]
        user_take_sighash: String,
        #[arg(help = "Public nonce (hex)")]
        user_take_nonce: String,
        #[arg(help = "User take signature (hex)")]
        user_take_signature: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // hardcoded for now
    // Fairgate scripts are hard to configure in this regard, as they do a source .env on each script pointing to this default port
    let anvil_port = 8545u16;

    let mut tmp_anvil = Anvil::new().port(anvil_port).arg("--host").arg("0.0.0.0");

    let anvil_block_time = std::env::var("ANVIL_BLOCK_TIME")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);

    if anvil_block_time != 0 {
        tmp_anvil = tmp_anvil.block_time(anvil_block_time);
    }

    let anvil = tmp_anvil.spawn();

    let key = anvil.keys().get(0).expect("No key found").clone();
    let signer = LocalSigner::from_signing_key(key.into());

    let wallet = EthereumWallet::from(signer);
    let anvil_provider = ProviderBuilder::new()
        .wallet(wallet)
        .with_simple_nonce_management()
        .connect(anvil.ws_endpoint_url().as_str())
        .await
        .expect("Could not set up provider");

    // Spawn background thread to update BitVMX consumers
    let bitvmx_broker_server = BitVmxBrokerServer::new(9094);
    let bitvmx_executor = Arc::new(Mutex::new(bitvmx::Executor::new(bitvmx_broker_server)));
    {
        let bitvmx_executor = Arc::clone(&bitvmx_executor);
        thread::spawn(move || {
            loop {
                {
                    let mut executor = bitvmx_executor.lock().unwrap();
                    if let Err(e) = executor.try_recv() {
                        eprintln!("Error receiving BitVMX message: {e}");
                    }
                }
                thread::sleep(Duration::from_millis(250));
            }
        });
    }

    let mut lines = BufReader::new(io::stdin()).lines();

    println!("Connected to Blockchain @ {}", anvil.endpoint_url());
    let events_executor =
        events::Executor::new(anvil_provider, anvil.ws_endpoint_url().as_str()).await?;

    print_help()?;

    loop {
        println!();
        print!("> ");
        std::io::stdout().flush().expect("Failed to flush stdout");

        let line = match lines.next_line().await? {
            Some(l) => l.trim().to_string(),
            None => break, // EOF
        };

        // build argv: program name + words of the line
        let argv = std::iter::once("app")
            .chain(line.split_whitespace())
            .collect::<Vec<_>>();

        match Menu::try_parse_from(argv) {
            Ok(menu) => match menu {
                Menu::InvokeRequestAdvanceFunds => {
                    events_executor.request_advance_funds().await?;
                }
                Menu::InvokeAdvanceFunds { pegout_id } => {
                    events_executor.advance_funds(pegout_id).await?;
                }
                Menu::PeginTransactionFound { btc_tx_file } => {
                    let json_str = std::fs::read_to_string(&btc_tx_file).with_context(|| {
                        format!("Failed to read file: {}", btc_tx_file.display())
                    })?;

                    let tx: Transaction = serde_json::from_str(&json_str)
                        .context("Failed to parse BTC transaction JSON")?;

                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");

                    executor.send_pegin_transaction_found(tx)?;
                }
                Menu::PeginRequested {
                    block_hash,
                    btc_tx_file,
                    merkle_branch_path,
                    merkle_branch_hashes,
                } => {
                    let json_str = std::fs::read_to_string(&btc_tx_file).with_context(|| {
                        format!("Failed to read file: {}", btc_tx_file.display())
                    })?;

                    let tx: Transaction = serde_json::from_str(&json_str)
                        .context("Failed to parse BTC transaction JSON")?;

                    let merkle_hashes: Vec<String> = merkle_branch_hashes
                        .split(',')
                        .map(str::to_string)
                        .collect();

                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");

                    executor.send_pegin_requested_event(
                        block_hash,
                        tx,
                        merkle_branch_path,
                        merkle_hashes,
                    )?;
                }
                Menu::PeginAccepted {
                    block_hash,
                    btc_tx_file,
                    merkle_branch_path,
                    merkle_branch_hashes,
                } => {
                    let json_str = std::fs::read_to_string(&btc_tx_file).with_context(|| {
                        format!("Failed to read file: {}", btc_tx_file.display())
                    })?;

                    let tx: Transaction = serde_json::from_str(&json_str)
                        .context("Failed to parse BTC transaction JSON")?;

                    let merkle_hashes: Vec<String> = merkle_branch_hashes
                        .split(',')
                        .map(str::to_string)
                        .collect();

                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");

                    executor.send_pegin_accepted_event(
                        block_hash,
                        tx,
                        merkle_branch_path,
                        merkle_hashes,
                    )?;
                }
                Menu::PegoutRequested {
                    amount_in_wei,
                    usr_pub_key,
                } => {
                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");

                    executor.send_pegout_requested_event(amount_in_wei, usr_pub_key)?;
                }
                Menu::RegisterPegout {
                    block_hash,
                    btc_tx_file,
                    merkle_branch_path,
                    merkle_branch_hashes,
                } => {
                    let json_str = std::fs::read_to_string(&btc_tx_file).with_context(|| {
                        format!("Failed to read file: {}", btc_tx_file.display())
                    })?;

                    let tx: Transaction = serde_json::from_str(&json_str)
                        .context("Failed to parse BTC transaction JSON")?;

                    let merkle_hashes: Vec<String> = merkle_branch_hashes
                        .split(',')
                        .map(str::to_string)
                        .collect();

                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");

                    executor.send_register_pegout_event(
                        block_hash,
                        tx,
                        merkle_branch_path,
                        merkle_hashes,
                    )?;
                }
                Menu::PegoutAccepted {
                    flow_id,
                    committee_id,
                    user_take_txid,
                    user_take_sighash,
                    user_take_nonce,
                    user_take_signature,
                } => {
                    let flow_id = Uuid::parse_str(&flow_id)?;
                    let committee_id = Uuid::parse_str(&committee_id)?;
                    let user_take_txid = Txid::from_str(&user_take_txid)?;
                    let user_take_sighash = Vec::<u8>::from_hex(user_take_sighash)?;
                    let user_take_nonce = PubNonce::from_str(&user_take_nonce)?;
                    let user_take_signature = PartialSignature::from_str(&user_take_signature)?;

                    let pegout_accepted = PegOutAccepted {
                        committee_id,
                        user_take_txid,
                        user_take_sighash,
                        user_take_nonce,
                        user_take_signature,
                    };

                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");

                    executor.send_pegout_accepted_event(flow_id, pegout_accepted)?;
                }
                Menu::Exit => {
                    break;
                }
            },
            Err(err) => {
                eprintln!("Invalid command: {err}");
                print_help()?;
            }
        }
    }

    println!();
    println!("Goodbye!");

    Ok(())
}

fn print_help() -> Result<()> {
    Menu::command().print_help()?;
    println!();
    println!("Ctrl+C or `exit` to quit the service (and anvil)");
    println!(
        "If attached in Docker, use [Ctrl + P, then Ctrl + Q] to detach without stopping the container"
    );
    Ok(())
}
