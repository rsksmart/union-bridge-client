use anyhow::Result;
use bitvmxclient_mocking::Executor;
use clap::{Parser, Subcommand};
use common::msg_broker::broker::BrokerServer;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Debug, Parser)]
#[command(
    name = "Mock BitVMX Events",
    about = "CLI to generate mock BitVMX events"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(alias = "get_temporary_pegin_address")]
    GetTemporaryPeginAddress {
        rootstock_deposit_address: String,
        value: u64,
        btc_reimbursement_pub_key: String,
    },

    Exit,
}

fn main() -> Result<()> {
    let broker_server = BrokerServer::new(9094);
    let executor = Arc::new(Mutex::new(Executor::new(broker_server)));

    // Spawn background thread to update consumers
    {
        let executor = Arc::clone(&executor);
        thread::spawn(move || {
            loop {
                {
                    let mut executor = executor.lock().unwrap();
                    if let Err(e) = executor.update_consumers() {
                        eprintln!("Error updating consumers: {e}");
                    }
                }
                thread::sleep(Duration::from_secs(5));
            }
        });
    }

    // CLI loop
    let stdin = io::stdin();
    let mut line = String::new();

    println!("Mock BitVMX Events CLI");
    println!("Type `get_temporary_pegin_address <addr> <val> <pubkey>`");
    println!("or `exit` to quit.");

    loop {
        print!("> ");
        io::stdout().flush()?;
        line.clear();
        if stdin.read_line(&mut line)? == 0 {
            break; // EOF
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        let argv = std::iter::once("bridge-cli")
            .chain(input.split_whitespace())
            .collect::<Vec<_>>();

        match Cli::try_parse_from(&argv) {
            Ok(Cli { command }) => match command {
                Commands::GetTemporaryPeginAddress {
                    rootstock_deposit_address,
                    value,
                    btc_reimbursement_pub_key,
                } => {
                    let executor = executor.lock().unwrap();
                    executor.send_get_temporary_pegin_address_event(
                        rootstock_deposit_address,
                        value,
                        btc_reimbursement_pub_key,
                    )?;
                }
                Commands::Exit => break,
            },
            Err(err) => {
                println!("parse error: {err}");
            }
        }
    }

    println!("\nGoodbye!");
    Ok(())
}
