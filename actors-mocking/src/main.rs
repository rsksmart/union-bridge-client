use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use actors_mocking::{bitvmx, events};
use alloy_node_bindings::Anvil;
use alloy_provider::ProviderBuilder;
use alloy_provider::network::EthereumWallet;
use alloy_signer_local::LocalSigner;
use anyhow::{Context, Result};
use bitcoin::Transaction;
use clap::{CommandFactory, Parser};
use common::msg_broker::broker::BitVmxBrokerServer;
use tokio::io::{self, AsyncBufReadExt, BufReader};

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
