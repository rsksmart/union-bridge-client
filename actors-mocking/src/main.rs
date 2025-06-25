use actors_mocking::{bitvmx, events};
use alloy_node_bindings::Anvil;
use alloy_provider::ProviderBuilder;
use alloy_provider::network::EthereumWallet;
use alloy_signer_local::LocalSigner;
use anyhow::Result;
use clap::{CommandFactory, Parser};
use common::msg_broker::broker::BrokerServer;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::io;
use tokio::io::{AsyncBufReadExt, BufReader};

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
    #[command(visible_alias = "gta")]
    RecvGetTemporaryPeginAddress {
        rootstock_deposit_address: String,
        value: u64,
        btc_reimbursement_pub_key: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let anvil_port = 2222u16;

    let anvil = Anvil::new()
        .block_time(1) // block every 1 seconds
        .port(anvil_port)
        .arg("--host")
        .arg("0.0.0.0") // for Docker networking compatibility
        .spawn();

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
    let broker_server = BrokerServer::new(9094);
    let bitvmx_executor = Arc::new(Mutex::new(bitvmx::Executor::new(broker_server)));
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
                thread::sleep(Duration::from_secs(5));
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
                Menu::RecvGetTemporaryPeginAddress {
                    rootstock_deposit_address,
                    value,
                    btc_reimbursement_pub_key,
                } => {
                    let executor = bitvmx_executor
                        .lock()
                        .expect("Failed to lock bitvmx_executor");
                    executor.send_get_temporary_pegin_address_event(
                        rootstock_deposit_address,
                        value,
                        btc_reimbursement_pub_key,
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
