use alloy_primitives::hex;
use alloy_provider::{ProviderBuilder, WsConnect, network::EthereumWallet};
use alloy_signer::k256::ecdsa::SigningKey;
use alloy_signer_local::LocalSigner;
use anyhow::Result;
use clap::{CommandFactory, Parser};
use mocks::events;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

#[derive(Debug, Parser)]
#[command(
    name = "Mock Union Bridge Contracts and BitVMX Client",
    about = "This CLI allows us to emit mocked Union Bridge contract events (invoking mocked UB Contracts)",
    arg_required_else_help = true
)]
enum Menu {
    /// exit the CLI
    Exit,

    /// emits RequestAdvanceFunds event, which triggers coordinator to start monitoring blocks for advance funds event
    #[command(
        visible_alias = "raf",
        about = "start monitoring blocks for advance funds (emits RequestAdvanceFunds event)"
    )]
    InvokeRequestAdvanceFunds,

    /// generate a fake advance-funds event that triggers the advance_funds_processor flow
    #[command(
        visible_alias = "kaf",
        about = "generate a fake advance-funds event that triggers the advance_funds_processor flow. Usage: kaf <PEGOUT_ID>"
    )]
    InvokeAdvanceFunds {
        #[arg(help = "the ID of the pegout to advance funds for")]
        pegout_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // use anvil default account
    const ANVIL_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    println!("Using Anvil default signer");

    // decode the hex private key (strip 0x prefix if present)
    let private_key_hex = ANVIL_PRIVATE_KEY
        .strip_prefix("0x")
        .unwrap_or(ANVIL_PRIVATE_KEY);
    let private_key_bytes = hex::decode(private_key_hex)?;

    // create signing key from private key bytes
    let signing_key = SigningKey::from_slice(&private_key_bytes)?;
    let signer = LocalSigner::from_signing_key(signing_key);

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
    let cmd = Menu::command();

    // print main header
    println!("{}", cmd.get_name());
    if let Some(about) = cmd.get_about() {
        println!("{}\n", about);
    }

    println!("Commands:");

    // iterate through subcommands and print with usage
    for subcommand in cmd.get_subcommands() {
        // get the usage string from clap
        let usage = subcommand.clone().render_usage().to_string();

        // extract just the command part (remove "Usage: " prefix if present)
        let usage_line = usage
            .strip_prefix("Usage: ")
            .unwrap_or(&usage)
            .split('\n')
            .next()
            .unwrap_or(&usage);

        println!("  {}", usage_line);

        if let Some(about) = subcommand.get_about() {
            println!("      {}", about);
        }
        println!();
    }

    println!("Ctrl+C or `exit` to quit the service (and anvil)");
    println!(
        "If attached in Docker, use [Ctrl + P, then Ctrl + Q] to detach without stopping the container"
    );
    Ok(())
}
