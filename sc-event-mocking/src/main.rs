use alloy_node_bindings::Anvil;
use alloy_provider::ProviderBuilder;
use alloy_provider::network::EthereumWallet;
use alloy_signer_local::LocalSigner;
use anyhow::Result;
use clap::{CommandFactory, Parser};
use sc_event_mocking::Executor;
use std::io::Write;
use tokio::io;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Debug, Parser)]
#[command(
    name = "Mock Union Bridge Events",
    about = "CLI to generate Mock Union Bridge Events",
    arg_required_else_help = true
)]
enum Menu {
    Exit,

    #[command(visible_alias = "raf")]
    RequestAdvanceFunds,

    #[command(visible_alias = "kaf")]
    KickoffAdvanceFunds {
        #[arg(help = "The ID of the pegout")]
        pegout_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let anvil_port = 2222u16;

    let anvil = Anvil::new()
        .block_time(1) // block every 1 seconds
        .port(anvil_port)
        .spawn();

    let key = anvil.keys().get(0).expect("No key found").clone();
    let signer = LocalSigner::from_signing_key(key.into());

    let wallet = EthereumWallet::from(signer);
    let anvil_provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect(anvil.ws_endpoint_url().as_str())
        .await
        .expect("Could not set up provider");

    let mut lines = BufReader::new(io::stdin()).lines();

    println!("Connected to Anvil @ {}", anvil.endpoint_url());
    let executor = Executor::new(anvil_provider, anvil.ws_endpoint_url().as_str()).await?;

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
                Menu::RequestAdvanceFunds => {
                    executor.request_advance_funds().await?;
                }
                Menu::KickoffAdvanceFunds { pegout_id } => {
                    executor.kickoff_advance_funds(pegout_id).await?;
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
