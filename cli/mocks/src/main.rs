use std::env;
use std::str::FromStr;

use alloy_primitives::{Address, hex};
use alloy_provider::{ProviderBuilder, WsConnect, network::EthereumWallet};
use alloy_signer::k256::ecdsa::SigningKey;
use alloy_signer_local::LocalSigner;
use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use mocks::events;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

const DEFAULT_RPC_URL: &str = "ws://127.0.0.1:8545";
const ANVIL_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const MOCKS_PRIVATE_KEY_ENV: &str = "MOCKS_PRIVATE_KEY";
const FAKE_PEG_MANAGER_ADDRESS_ENV: &str = "FAKE_PEG_MANAGER_ADDRESS";

#[derive(Debug, Parser)]
#[command(
    name = "Mock Union Bridge Contracts and BitVMX Client",
    about = "This CLI emits mocked Union Bridge contract events (FakePegManager)",
    arg_required_else_help = false
)]
struct Cli {
    #[arg(long, default_value = DEFAULT_RPC_URL, help = "Rootstock WebSocket RPC URL")]
    rpc_url: String,
    #[arg(long, help = "Predeployed FakePegManager address for attach mode")]
    fake_peg_manager_address: Option<String>,
    #[arg(
        long,
        default_value_t = false,
        help = "Attach to existing FakePegManager instead of deploying"
    )]
    no_deploy: bool,
}

#[derive(Debug, Parser)]
#[command(
    name = "Mock Union Bridge Contracts and BitVMX Client",
    about = "Interactive commands for FakePegManager events",
    arg_required_else_help = true
)]
enum Menu {
    /// exit the CLI
    Exit,

    /// emits RequestAdvanceFunds event, which triggers coordinator to start monitoring blocks for advance funds event
    #[command(
        visible_alias = "raf",
        about = "start monitoring blocks for advance funds (emits RequestAdvanceFunds event). Usage: raf [PEGOUT_ID]"
    )]
    InvokeRequestAdvanceFunds {
        #[arg(help = "optional pegout id to reuse for subsequent kaf", required = false)]
        pegout_id: Option<String>,
    },

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
    let cli = Cli::parse();
    let private_key = resolve_private_key();
    let fake_peg_manager_address = resolve_fake_peg_manager_address(cli.fake_peg_manager_address)?;

    // decode the hex private key (strip 0x prefix if present)
    let private_key_hex = private_key.strip_prefix("0x").unwrap_or(&private_key);
    let private_key_bytes = hex::decode(private_key_hex).context("invalid private key hex")?;

    // create signing key from private key bytes
    let signing_key = SigningKey::from_slice(&private_key_bytes)?;
    let signer = LocalSigner::from_signing_key(signing_key);
    let signer_address = signer.address();

    let ws = WsConnect::new(&cli.rpc_url);

    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .with_simple_nonce_management()
        .connect_ws(ws)
        .await
        .context("Could not set up provider")?;

    let mut rl = DefaultEditor::new()?;

    println!("Connected to blockchain @ {}", cli.rpc_url);
    println!("Using signer {}", signer_address);
    if cli.no_deploy {
        println!("Mode: attach (--no-deploy)");
    } else {
        println!("Mode: deploy (default)");
    }

    let events_executor =
        events::Executor::new(provider, &cli.rpc_url, fake_peg_manager_address, cli.no_deploy)
            .await?;

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
        let argv = std::iter::once("app").chain(line.split_whitespace()).collect::<Vec<_>>();

        match Menu::try_parse_from(argv) {
            Ok(menu) => match menu {
                Menu::InvokeRequestAdvanceFunds { pegout_id } => {
                    events_executor.request_advance_funds(pegout_id).await?;
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

fn resolve_private_key() -> String {
    if let Ok(value) = env::var(MOCKS_PRIVATE_KEY_ENV) {
        return value;
    }

    println!(
        "No private key provided. Falling back to anvil default key. For regtest set {}.",
        MOCKS_PRIVATE_KEY_ENV
    );
    ANVIL_PRIVATE_KEY.to_string()
}

fn resolve_fake_peg_manager_address(address_arg: Option<String>) -> Result<Option<Address>> {
    let raw = address_arg.or_else(|| env::var(FAKE_PEG_MANAGER_ADDRESS_ENV).ok());
    raw.map(|value| {
        Address::from_str(&value).with_context(|| {
            format!(
                "invalid fake peg manager address `{value}`. Pass --fake-peg-manager-address or {}",
                FAKE_PEG_MANAGER_ADDRESS_ENV
            )
        })
    })
    .transpose()
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
        let usage_line =
            usage.strip_prefix("Usage: ").unwrap_or(&usage).split('\n').next().unwrap_or(&usage);

        println!("  {}", usage_line);

        if let Some(about) = subcommand.get_about() {
            println!("      {}", about);
        }
        println!();
    }

    println!("Ctrl+C or `exit` to quit the service");
    println!(
        "If attached in Docker, use [Ctrl + P, then Ctrl + Q] to detach without stopping the container"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_no_deploy_requires_external_address_later() {
        let parsed = Cli::parse_from(["mocks", "--no-deploy"]);
        assert!(parsed.no_deploy);
        assert!(parsed.fake_peg_manager_address.is_none());
    }

    #[test]
    fn test_cli_parses_regtest_attach_flags() {
        let parsed = Cli::parse_from([
            "mocks",
            "--rpc-url",
            "ws://example-regtest-node:4445",
            "--fake-peg-manager-address",
            "0x7Cd31D33302B6f5Bc45763487195Ae329a915beB",
            "--no-deploy",
        ]);

        assert_eq!(parsed.rpc_url, "ws://example-regtest-node:4445");
        assert!(parsed.no_deploy);
        assert_eq!(
            parsed.fake_peg_manager_address.as_deref(),
            Some("0x7Cd31D33302B6f5Bc45763487195Ae329a915beB")
        );
    }
}
