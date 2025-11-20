use actors_mocking::events;
use alloy_provider::{ProviderBuilder, WsConnect, network::EthereumWallet};
use anyhow::Result;
use clap::{CommandFactory, Parser};
use key_manager;
use key_manager::key_manager::KeyManager;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::path::PathBuf;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let base_storage_path = std::env::var("BASE_STORAGE_PATH").expect("BASE_STORAGE_PATH not set");
    let key_path = PathBuf::from(format!(
        "{}/.union_bridge/keystore/multi-client-1-user",
        base_storage_path
    ));

    let signer = KeyManager::get_signer(&key_path)?;

    let ws = WsConnect::new("ws://127.0.0.1:8545");

    let wallet = EthereumWallet::from(signer);
    let anvil_provider = ProviderBuilder::new()
        .wallet(wallet)
        .with_simple_nonce_management()
        .connect(ws.url())
        .await
        .expect("Could not set up provider");

    let mut rl = DefaultEditor::new()?;

    println!("Connected to Blockchain @ {}", ws.url().to_string());
    let events_executor = events::Executor::new(anvil_provider, ws.url()).await?;

    print_help()?;

    loop {
        let readline = rl.readline("\n> ");

        let line = match readline {
            Ok(line) => {
                rl.add_history_entry(&line)?;
                line.trim().to_string()
            }
            Err(ReadlineError::Interrupted) => {
                // ctrl-c
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                // ctrl-d
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        };

        if line.is_empty() {
            continue;
        }

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
