use std::borrow::Cow;
use std::convert::TryFrom;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::address::{Address, NetworkUnchecked};
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::hashes::hex::FromHex;
use bitcoin::key::PublicKey;
use bitcoin::network::Network;
use bitcoin::{OutPoint, ScriptBuf, Txid};
use chrono::{DateTime, Utc};
use clap::Parser;
use rustyline::error::ReadlineError;
use ub_wallet::bitcoin::utils::{send_test_funds, start_client};
use ub_wallet::cli::{CliOpts, setup_editor};
use ub_wallet::config::{self, Config};
use ub_wallet::wallet::{CreatedTransaction, Wallet};

fn main() -> Result<()> {
    let opts = CliOpts::parse();
    let (config, config_path) = Config::load(&opts)?;
    let (history_path, mut editor) = setup_editor(opts, config_path.clone())?;

    if let Some(path) = config_path.as_ref() {
        println!("Loaded config from {}", path.display());
    }

    let mut wallet = Wallet::from_config(&config)?;

    println!(
        "Simple P2WPKH wallet (network: {}). Type 'help' for commands.",
        wallet.network()
    );

    loop {
        let prompt = prompt_for(wallet.network());
        match editor.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let history_entry: Cow<'_, str> = match trimmed.split_whitespace().next() {
                    Some("import_private_key") => {
                        Cow::Owned("import_private_key <redacted>".to_string())
                    }
                    _ => Cow::Borrowed(trimmed),
                };

                let _ = editor.add_history_entry(history_entry.as_ref());

                match handle_command(&mut wallet, &config, trimmed) {
                    Ok(CommandOutcome::Continue) => {}
                    Ok(CommandOutcome::Exit) => break,
                    Err(err) => eprintln!("Error: {:#}", err),
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("Input error: {err}");
                break;
            }
        }
    }

    if let Err(err) = editor.save_history(&history_path) {
        eprintln!("Failed to persist command history: {err}");
    }

    Ok(())
}

enum CommandOutcome {
    Continue,
    Exit,
}

fn handle_command(wallet: &mut Wallet, config: &Config, line: &str) -> Result<CommandOutcome> {
    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap();

    match command {
        "help" => {
            print_help(wallet.sats_per_byte());
            Ok(CommandOutcome::Continue)
        }
        "exit" | "quit" => Ok(CommandOutcome::Exit),
        "set_network" => {
            let name = parts
                .next()
                .context("expected network name (bitcoin|testnet|signet|regtest)")?;
            let network = config::parse_network(name)?;
            let had_in_memory_private_keys = !wallet.imported_addresses().is_empty();
            let previous_network = wallet.network();
            let changed = wallet.set_network(network)?;
            if changed {
                println!(
                    "Network set to {}. In-memory state reset; UTXO data is per network.",
                    network_name(wallet.network())
                );
                if let Some(wif) = config.private_key_wif.as_deref() {
                    if matches!(
                        wallet.network(),
                        Network::Regtest | Network::Testnet | Network::Testnet4 | Network::Signet
                    ) {
                        let address = wallet
                            .import_private_key(wif)
                            .with_context(|| "failed to load private key from config")?;
                        println!(
                            "Loaded private key from config. Default P2WPKH address: {address}"
                        );
                    }
                }
                if wallet.network() == Network::Bitcoin && had_in_memory_private_keys {
                    println!(
                        "Unloaded in-memory {} private keys before connecting to mainnet.",
                        network_name(previous_network)
                    );
                }
            } else {
                println!("Network unchanged ({}).", network_name(wallet.network()));
            }
            Ok(CommandOutcome::Continue)
        }
        "import_private_key" => {
            let wif = parts.next().context("expected WIF string")?;
            let address = wallet.import_private_key(wif)?;
            println!("Imported key. Default P2WPKH address: {address}");
            Ok(CommandOutcome::Continue)
        }
        "generate_address" => {
            let generated = wallet.generate_address()?;
            let address_str = generated.address.to_string();
            println!("Generated P2WPKH address: {}", address_str);
            println!("  Private key (WIF): {}", generated.private_key_wif);
            println!("  Public key: {}", generated.public_key_hex);
            let is_active = wallet
                .active_address()
                .map(|addr| addr.to_string() == address_str)
                .unwrap_or(false);
            if !is_active {
                println!(
                    "  (active address unchanged; use switch_address {address_str} to activate)"
                );
            }
            Ok(CommandOutcome::Continue)
        }
        "list_addresses" => {
            let addresses = wallet.imported_addresses();
            if addresses.is_empty() {
                println!("No addresses imported.");
            } else {
                println!("Imported addresses:");
                let active = wallet.active_address().map(|addr| addr.to_string());
                for addr in addresses {
                    if active.as_deref() == Some(addr.as_str()) {
                        println!("  {addr} (active)");
                    } else {
                        println!("  {addr}");
                    }
                }
            }
            Ok(CommandOutcome::Continue)
        }
        "switch_address" => {
            let address_str = parts.next().context("expected bech32 address")?;
            let address_unchecked: Address<NetworkUnchecked> =
                Address::from_str(address_str).context("invalid address format")?;
            let checked = address_unchecked
                .require_network(wallet.network())
                .map_err(|_| {
                    anyhow!(
                        "address does not match current network {:?}",
                        wallet.network()
                    )
                })?;
            wallet.switch_active_address(checked.clone())?;
            println!("Active address set to {checked}");
            Ok(CommandOutcome::Continue)
        }
        "set_rpc" => {
            let url = parts.next().context("expected RPC URL")?;
            let user = parts.next();
            let pass = parts.next();
            wallet.configure_rpc(url, user, pass)?;
            println!("RPC client configured (URL: {url}).");
            Ok(CommandOutcome::Continue)
        }
        "clear_rpc" => {
            wallet.clear_rpc_client();
            println!("RPC client cleared.");
            Ok(CommandOutcome::Continue)
        }
        "start_regtest_client" => {
            let client = start_client()?;
            wallet.set_rpc_client(client);
            println!("Regtest RPC client started and configured.");
            Ok(CommandOutcome::Continue)
        }
        "register_utxo" => {
            if wallet.active_address().is_none() {
                bail!("import or switch to an address before registering UTXOs");
            }
            let txid_str = parts.next().context("expected txid")?;
            let vout_str = parts.next().context("expected vout")?;
            let amount = match parts.next() {
                Some(value) => Some(value.parse().context("invalid amount (satoshis)")?),
                None => None,
            };

            let txid = Txid::from_str(txid_str).context("invalid txid provided")?;
            let vout: u32 = vout_str.parse().context("invalid vout index")?;

            let amount_sat = match amount {
                Some(value) => value,
                None => wallet
                    .fetch_utxo_amount(txid, vout)
                    .context("RPC client required to fetch UTXO amount")?,
            };

            let outpoint = OutPoint::new(txid, vout);
            wallet.register_utxo(outpoint, amount_sat)?;
            println!("Registered UTXO {} with value {} sat", outpoint, amount_sat);
            Ok(CommandOutcome::Continue)
        }
        "list_funds" => {
            let scope = parts.next();
            if let Some(extra) = parts.next() {
                bail!("unexpected argument '{extra}' for list_funds");
            }

            match scope {
                Some("all") => {
                    let entries = wallet.utxos_with_timestamps_all()?;
                    print_all_utxos(wallet, entries);
                }
                Some(other) => bail!("invalid list_funds argument '{other}'. Use 'all'."),
                None => {
                    print_active_address_utxos(wallet)?;
                }
            }
            Ok(CommandOutcome::Continue)
        }
        "send_to_pubkey" => {
            let pubkey_hex = parts.next().context("expected public key hex")?;
            let amount_str = parts.next().context("expected amount in satoshis")?;
            let amount: u64 = amount_str.parse().context("invalid amount (satoshis)")?;
            let count = parse_count(parts.next())?;

            let pubkey_bytes = Vec::from_hex(pubkey_hex).context("public key must be hex")?;
            let pk = PublicKey::from_slice(&pubkey_bytes).context("invalid public key")?;
            let wpkh = pk
                .wpubkey_hash()
                .map_err(|_| anyhow!("public key must be compressed for P2WPKH"))?;
            let target_script = ScriptBuf::new_p2wpkh(&wpkh);

            let created = wallet.create_transactions(target_script, amount, count)?;
            print_transactions(&created);
            maybe_broadcast(wallet, &created)?;
            Ok(CommandOutcome::Continue)
        }
        "send_to_address" => {
            let address_str = parts.next().context("expected bech32 address")?;
            let amount_str = parts.next().context("expected amount in satoshis")?;
            let amount: u64 = amount_str.parse().context("invalid amount (satoshis)")?;
            let count = parse_count(parts.next())?;

            let address: Address<NetworkUnchecked> =
                Address::from_str(address_str).context("invalid address format")?;
            let checked = address.require_network(wallet.network()).map_err(|_| {
                anyhow!(
                    "address does not match current network {:?}",
                    wallet.network()
                )
            })?;
            let script = checked.script_pubkey();
            if !script.is_p2wpkh() {
                bail!("only native segwit (P2WPKH) addresses are supported");
            }

            let created = wallet.create_transactions(script, amount, count)?;
            print_transactions(&created);
            maybe_broadcast(wallet, &created)?;
            Ok(CommandOutcome::Continue)
        }
        "send_test_funds" => {
            send_test_funds(wallet)?;
            Ok(CommandOutcome::Continue)
        }

        other => Err(anyhow!(
            "unknown command '{other}'. Type 'help' for a list of commands."
        )),
    }
}

fn prompt_for(network: Network) -> String {
    let name = network_name(network);
    let prompt_color = "\x1b[36m";
    let network_color = match network {
        Network::Bitcoin => "\x1b[31m",
        _ => prompt_color,
    };
    let reset = "\x1b[0m";
    format!("{prompt_color}ub-wallet ({network_color}{name}{prompt_color})>{reset} ",)
}

fn network_name(network: Network) -> &'static str {
    match network {
        Network::Bitcoin => "bitcoin",
        Network::Testnet => "testnet",
        Network::Testnet4 => "testnet4",
        Network::Signet => "signet",
        Network::Regtest => "regtest",
    }
}

fn print_active_address_utxos(wallet: &mut Wallet) -> Result<(), anyhow::Error> {
    Ok(if let Some(address) = wallet.active_address() {
        let utxos = wallet.utxos_with_timestamps()?;
        println!("Registered UTXOs for {address}:");
        if utxos.is_empty() {
            println!("  (no registered UTXOs)");
        } else {
            print_utxos(&utxos);
        }
    } else {
        println!("No active address. Import or switch to an address to list funds.");
    })
}

fn print_all_utxos(
    wallet: &mut Wallet,
    entries: Vec<(Address, Vec<(ub_wallet::wallet::Utxo, u64)>)>,
) {
    if entries.is_empty() {
        println!("No addresses imported.");
    } else {
        let active = wallet.active_address().map(|a| a.to_string());
        for (i, (address, utxos)) in entries.iter().enumerate() {
            let is_active = active.as_deref() == Some(address.to_string().as_str());
            let prefix = if is_active {
                "\x1b[33mRegistered UTXOs for (active) "
            } else {
                "Registered UTXOs for "
            };
            println!("{prefix}{}:\x1b[0m", address);
            if utxos.is_empty() {
                println!("  (no registered UTXOs)");
            } else {
                print_utxos(utxos);
            }
            if i + 1 < entries.len() {
                println!();
            }
        }
    }
}

fn print_utxos(utxos: &Vec<(ub_wallet::wallet::Utxo, u64)>) {
    for (utxo, ts) in utxos {
        println!(
            "  {} -> {} sat (added at {})",
            utxo.outpoint,
            utxo.value_sat,
            format_timestamp(*ts)
        );
    }
}

fn parse_count(raw: Option<&str>) -> Result<usize> {
    match raw {
        None => Ok(1),
        Some(value) => {
            let count: usize = value.parse().context("count must be a positive integer")?;
            if count == 0 {
                bail!("count must be at least 1");
            }
            Ok(count)
        }
    }
}

fn print_transactions(txs: &[CreatedTransaction]) {
    for (idx, created) in txs.iter().enumerate() {
        let tx = &created.transaction;
        let txid = tx.compute_txid();
        let vsize = tx.vsize();
        let hex = serialize_hex(tx);
        println!(
            "Transaction {}: txid={} vsize={} fee={} sat raw={}",
            idx + 1,
            txid,
            vsize,
            created.fee_sat,
            hex
        );

        if let Some(change) = &created.change {
            println!(
                "  change -> {} sat back to wallet (outpoint {}:{})",
                change.value_sat, change.outpoint.txid, change.outpoint.vout
            );
        }
    }
}

fn maybe_broadcast(wallet: &Wallet, txs: &[CreatedTransaction]) -> Result<()> {
    if wallet.rpc_client().is_none() {
        println!("RPC not configured; transaction hex printed only.");
        return Ok(());
    }

    for (idx, created) in txs.iter().enumerate() {
        match wallet.broadcast_transaction(created) {
            Ok(txid) => println!("  broadcasted tx {} -> {}", idx + 1, txid),
            Err(err) => eprintln!("  failed to broadcast tx {}: {err}", idx + 1),
        }
    }
    Ok(())
}

fn print_help(sats_per_byte: u64) {
    println!("Available commands:");
    println!("  help                                  - Show this message");
    println!("  exit | quit                           - Leave the wallet");
    println!(
        "  set_network <name>                    - Select network (bitcoin|testnet|testnet4|signet|regtest)"
    );
    println!(
        "  import_private_key <wif>              - Import compressed WIF for the current network kind"
    );
    println!(
        "  generate_address                      - Create a new P2WPKH key pair without switching active address"
    );
    println!("  list_addresses                        - Show imported wallet addresses");
    println!(
        "  switch_address <addr>                 - Make an imported address the active wallet address"
    );
    println!("  set_rpc <url> [user] [pass]           - Configure RPC endpoint for broadcasting");
    println!("  clear_rpc                             - Remove configured RPC client");
    println!(
        "  start_regtest_client                  - Launch regtest bitcoind via Docker and configure RPC"
    );
    println!("  register_utxo <txid> <vout> [sats]    - Register a spendable P2WPKH UTXO");
    println!(
        "  list_funds [all]                      - List UTXOs for the active address or every address"
    );
    println!(
        "  send_to_pubkey <hex> <sats> [count]   - Create count (default 1) txs to a P2WPKH pubkey"
    );
    println!(
        "  send_to_address <addr> <sats> [count] - Create count (default 1) txs to a Bech32 address"
    );
    println!(
        "  send_test_funds                       - Regtest only: mine and fund the active wallet via RPC"
    );
    println!("Fees target {sats_per_byte} sat per virtual byte.");
}

fn format_timestamp(timestamp: u64) -> String {
    match i64::try_from(timestamp)
        .ok()
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
    {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => timestamp.to_string(),
    }
}
