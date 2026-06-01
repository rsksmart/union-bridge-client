#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::convert::TryFrom;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use bitcoin::address::{Address, NetworkUnchecked};
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::hashes::hex::FromHex;
use bitcoin::key::PublicKey;
use bitcoin::network::Network;
use bitcoin::{OutPoint, ScriptBuf, Txid};
use bitcoincore_rpc::RpcApi;
use chrono::{DateTime, Utc};
use clap::Parser;
use reqwest::blocking::Client;
use rustyline::error::ReadlineError;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Value, json};
use ub_wallet::bitcoin::utils::find_vout_for_address;
use ub_wallet::cli::{CliOpts, WalletMode, setup_editor};
use ub_wallet::config::Config;
use ub_wallet::wallet::{CreatedTransaction, Wallet, network_name};

fn main() -> Result<()> {
    let opts = CliOpts::parse();
    let (config, config_path) = Config::load(&opts)?;

    if let Some(path) = config_path.as_ref() {
        println!("Loaded config from {}", path.display());
    }

    let mut wallet = Wallet::from_config(&config).map_err(|err| {
        let err_msg = format!("{:#}", err);

        // detect database lock errors (RocksDB can't acquire lock)
        let is_lock_error = err_msg.to_lowercase().contains("lock")
            || err_msg.contains("Error creating storage")
            || err_msg.contains("IO error")
            || err_msg.contains("Resource temporarily unavailable");

        if is_lock_error {
            let network = config.network.unwrap_or(Network::Regtest);
            let network_suffix_str = network_name(network);
            let mode_name = config.mode.to_string();
            let utxo_db_path = config.db_path.join(&mode_name).join(network_suffix_str).join("utxo_db");
            let pending_tx_db_path = config.db_path.join(&mode_name).join(network_suffix_str).join("pending_tx_db");

            eprintln!("Error: Cannot open wallet - database is locked by another process.");
            eprintln!();
            eprintln!("This means another wallet instance ({} mode) is currently running.", mode_name);
            eprintln!();
            eprintln!("Common causes:");
            eprintln!("  1. Wallet is open in interactive mode in another terminal");
            eprintln!("  2. Another command is currently executing");
            eprintln!("  3. Previous instance crashed leaving stale locks");
            eprintln!();
            eprintln!("What to do:");
            eprintln!("  1. Close any interactive wallet sessions (type 'exit' or press Ctrl+D)");
            eprintln!("  2. Wait for running commands to complete");
            eprintln!("  3. Check for zombie processes:");
            eprintln!("     ps aux | grep 'ub-wallet.*--mode {}'", mode_name);
            eprintln!("  4. If nothing is running, remove stale LOCK files:");
            eprintln!("     rm -f {}/LOCK", utxo_db_path.display());
            eprintln!("     rm -f {}/LOCK", pending_tx_db_path.display());
            eprintln!();
            eprintln!("Note: User and member modes use separate databases, so both can run simultaneously.");
        } else {
            // not a lock error, show original error
            eprintln!("Error: {}", err_msg);
        }

        err
    })?;

    // check if a command was provided for non-interactive execution
    if !opts.command.is_empty() {
        // programmatic/command mode is only allowed in regtest for safety
        if wallet.network() != Network::Regtest {
            eprintln!("Error: Command mode is only available on regtest network.");
            eprintln!();
            eprintln!("Current network: {:?}", wallet.network());
            eprintln!();
            eprintln!("Reason: Programmatic access is restricted to regtest for safety.");
            eprintln!("For testnet/mainnet operations, please use interactive mode:");
            let env_name = network_name(wallet.network());
            eprintln!("  ./scripts/bitcoin-wallet.sh {} --env {}", config.mode, env_name);
            bail!("Command mode not allowed on network: {:?}", wallet.network());
        }

        let command_line = opts.command.join(" ");
        return match handle_command(&mut wallet, &command_line, &config.mode) {
            Ok(_) => Ok(()),
            Err(err) => {
                eprintln!("Error: {:#}", err);
                bail!("Command failed");
            }
        };
    }

    // interactive mode
    // store history file in mode/network directory
    let network = config.network.unwrap_or(Network::Regtest);
    let network_name = network_name(network);
    let mode_name = config.mode.to_string();
    let history_path = &config.db_path.join(&mode_name).join(network_name).join("cli_history");
    let mut editor = setup_editor(history_path)?;

    println!(
        "Simple P2WPKH wallet (mode: {}, network: {}). Type 'help' for commands.",
        config.mode,
        wallet.network()
    );

    loop {
        let prompt = prompt_for(&config.mode, network_name);
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

                match handle_command(&mut wallet, trimmed, &config.mode) {
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

#[derive(Debug, Clone)]
struct TxConfirmationInfo {
    confirmations: u64,
    block_hash: Option<String>,
    block_height: Option<i64>,
    total_value_btc: f64,
}

fn check_transaction_status(wallet: &Wallet, txid: &Txid) -> Result<TxConfirmationInfo> {
    let client = wallet.rpc_client().context("RPC client required to check transaction status")?;
    let verbose: serde_json::Value = client
        .call("getrawtransaction", &[json!(txid.to_string()), json!(true)])
        .context("failed to fetch transaction details")?;

    let confirmations = verbose.get("confirmations").and_then(|c| c.as_u64()).unwrap_or(0);

    let total_value_btc: f64 = verbose
        .get("vout")
        .and_then(|outs| outs.as_array())
        .map(|outs| {
            outs.iter().filter_map(|out| out.get("value").and_then(|val| val.as_f64())).sum()
        })
        .unwrap_or(0.0);

    let block_hash = verbose.get("blockhash").and_then(|h| h.as_str()).map(|s| s.to_string());

    let block_height = if let Some(ref hash) = block_hash {
        let block_json: serde_json::Value =
            client.call("getblock", &[json!(hash)]).context("failed to fetch block information")?;
        block_json.get("height").and_then(|h| h.as_i64())
    } else {
        None
    };

    Ok(TxConfirmationInfo { confirmations, block_hash, block_height, total_value_btc })
}

fn require_operational_mode(mode: &WalletMode, cmd: &str) -> Result<()> {
    if matches!(mode, WalletMode::Default) {
        bail!(
            "Command '{}' is not available in default mode. \
             Please add your WIF as an environment variable and run with --mode user or --mode member.",
            cmd
        );
    }
    Ok(())
}

fn handle_command(wallet: &mut Wallet, line: &str, mode: &WalletMode) -> Result<CommandOutcome> {
    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap();

    match command {
        "help" => {
            print_help(wallet.sats_per_byte(), mode);
            Ok(CommandOutcome::Continue)
        }
        "exit" | "quit" => Ok(CommandOutcome::Exit),
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
            println!("  Private key (WIF): {}", generated.private_key_wif.expose_secret());
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
            require_operational_mode(mode, command)?;
            let address_str = parts.next().context("expected bech32 address")?;
            let address_unchecked: Address<NetworkUnchecked> =
                Address::from_str(address_str).context("invalid address format")?;
            let checked = address_unchecked.require_network(wallet.network()).map_err(|_| {
                anyhow!("address does not match current network {:?}", wallet.network())
            })?;
            wallet.switch_active_address(checked.clone())?;
            println!("Active address set to {checked}");
            Ok(CommandOutcome::Continue)
        }
        "register_utxo" => {
            require_operational_mode(mode, command)?;
            if wallet.active_address().is_none() {
                bail!("import or switch to an address before registering UTXOs");
            }
            let txid_str = parts.next().context("expected txid")?;
            let block_hash_str = parts.next().context("expected block hash")?;
            let vout_str = parts.next().context("expected vout")?;
            let amount = match parts.next() {
                Some(value) => Some(value.parse().context("invalid amount (satoshis)")?),
                None => None,
            };

            let txid = Txid::from_str(txid_str).context("invalid txid provided")?;
            let block_hash = bitcoincore_rpc::bitcoin::BlockHash::from_str(block_hash_str)
                .context("invalid block hash")?;
            let vout: u32 = vout_str.parse().context("invalid vout index")?;

            let amount_sat = match amount {
                Some(value) => value,
                None => wallet
                    .fetch_utxo_amount(txid, Some(&block_hash), vout)
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
            require_operational_mode(mode, command)?;
            // Syntax: send_to_pubkey <hex_csv> <satoshis> [count]
            // <hex_csv> is a comma-separated list of compressed public keys in hex (no spaces)
            let hex_csv =
                parts.next().context("expected comma-separated list of public keys (hex)")?;
            let amount_str = parts.next().context("expected amount in satoshis")?;
            let amount: u64 = amount_str.parse().context("invalid amount (satoshis)")?;
            let count = parse_count(parts.next())?;

            let pubkeys: Vec<&str> =
                hex_csv.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            ensure!(
                !pubkeys.is_empty(),
                "expected at least one public key (hex) in the comma-separated list"
            );

            let mut scripts: Vec<ScriptBuf> = Vec::new();
            for pk_hex in pubkeys {
                let pubkey_bytes = Vec::from_hex(pk_hex).context("public key must be hex")?;
                let pk = PublicKey::from_slice(&pubkey_bytes).context("invalid public key")?;
                let wpkh = pk
                    .wpubkey_hash()
                    .map_err(|_| anyhow!("public key must be compressed for P2WPKH"))?;
                let target_script = ScriptBuf::new_p2wpkh(&wpkh);
                scripts.push(target_script);
            }

            let created = wallet.create_transactions(scripts, amount, count)?;
            print_transactions(&created);
            maybe_broadcast(wallet, &created)?;
            Ok(CommandOutcome::Continue)
        }
        "send_to_address" => {
            require_operational_mode(mode, command)?;
            // Syntax: send_to_address <addr_csv> <satoshis> [count]
            // <addr_csv> is a comma-separated list of recipient addresses (no spaces)
            // Supports: Bech32 P2WPKH and Base58 P2PKH on the current network
            let addr_csv = parts.next().context("expected comma-separated address list")?;
            let amount_str = parts.next().context("expected amount in satoshis")?;
            let amount: u64 = amount_str.parse().context("invalid amount (satoshis)")?;
            let count = parse_count(parts.next())?;

            let addresses: Vec<&str> =
                addr_csv.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            ensure!(
                !addresses.is_empty(),
                "expected at least one address in the comma-separated list"
            );

            let mut scripts: Vec<ScriptBuf> = Vec::new();
            for address_str in addresses {
                let address: Address<NetworkUnchecked> =
                    Address::from_str(address_str).context("invalid address format")?;
                let checked = address.require_network(wallet.network()).map_err(|_| {
                    anyhow!("address does not match current network {:?}", wallet.network())
                })?;
                let script = checked.script_pubkey();
                if !(script.is_p2wpkh() || script.is_p2pkh()) {
                    bail!("only P2WPKH (bech32) and P2PKH (base58) addresses are supported");
                }
                scripts.push(script);
            }

            let created = wallet.create_transactions(scripts, amount, count)?;

            print_transactions(&created);
            maybe_broadcast(wallet, &created)?;
            Ok(CommandOutcome::Continue)
        }
        "mine_block" => {
            require_operational_mode(mode, command)?;
            ensure!(
                wallet.network() == Network::Regtest,
                "mine_block is only available on regtest (current: {:?})",
                wallet.network()
            );
            let Some(client) = wallet.rpc_client() else {
                println!(
                    "RPC not configured; mining requires an RPC node. Configure RPC in config (rpc_url) or via env."
                );
                return Ok(CommandOutcome::Continue);
            };
            let miner_address: String = client
                .call("getnewaddress", &[json!("miner"), json!("bech32")])
                .context("failed to obtain mining address")?;
            let blocks: Vec<String> = client
                .call("generatetoaddress", &[json!(1), json!(miner_address.clone())])
                .context("failed to mine block on regtest")?;
            if let Some(hash) = blocks.first() {
                let block_json: serde_json::Value = client
                    .call("getblock", &[json!(hash)])
                    .context("failed to fetch block info")?;
                if let Some(height) = block_json.get("height").and_then(|v| v.as_i64()) {
                    println!("Mined block {} at height {}", hash, height);
                } else {
                    println!("Mined block {}", hash);
                }
            } else {
                println!("No block hash returned by generatetoaddress");
            }
            Ok(CommandOutcome::Continue)
        }
        "mine_utxo" => {
            require_operational_mode(mode, command)?;
            ensure!(
                wallet.network() == Network::Regtest,
                "mine_utxo is only available on regtest (current: {:?})",
                wallet.network()
            );
            let Some(active_addr) = wallet.active_address().cloned() else {
                println!("No active address. Import or switch to an address before mining a UTXO.");
                return Ok(CommandOutcome::Continue);
            };
            let Some(client) = wallet.rpc_client() else {
                println!(
                    "RPC not configured; mining requires an RPC node. Configure RPC in config (rpc_url) or via env."
                );
                return Ok(CommandOutcome::Continue);
            };

            // Optional amount in satoshis (default to 21_000_000 sat = 0.21 BTC)
            let amount_sat: u64 = match parts.next() {
                Some(s) => s.parse().context("invalid amount (satoshis)")?,
                None => 21_000_000,
            };
            let send_amount_btc = sats_to_btc(amount_sat);
            let required_balance_sat = amount_sat.saturating_add(MINER_FEE_BUFFER_SAT);

            let miner_address: String = client
                .call("getnewaddress", &[json!("miner"), json!("bech32")])
                .context("failed to obtain mining address")?;

            // `--start-blockchains` should have already bootstrapped mature miner funds; this only
            // falls back to the 101-block regtest bootstrap when the wallet still cannot fund the send.
            if miner_trusted_balance_sat(client)? < required_balance_sat {
                eprintln!(
                    "Warning: miner wallet balance is below {} sat; falling back to 101-block regtest bootstrap.",
                    required_balance_sat
                );
                // Bootstrap regtest funds only when the miner wallet cannot already cover this send.
                client
                    .call::<Vec<String>>(
                        "generatetoaddress",
                        &[json!(101), json!(miner_address.clone())],
                    )
                    .context("failed to pre-mine regtest blocks")?;
            }

            // Send requested amount to the active address
            let txid_hex: String = client
                .call("sendtoaddress", &[json!(active_addr.to_string()), json!(send_amount_btc)])
                .context("failed to fund active address")?;

            // Mine one block to confirm the transaction
            client
                .call::<Vec<String>>("generatetoaddress", &[json!(1), json!(miner_address)])
                .context("failed to confirm funding transaction")?;

            // Find the vout for our address and register the UTXO
            let funding_txid =
                Txid::from_str(&txid_hex).context("invalid txid returned by bitcoind")?;
            let funding_vout = find_vout_for_address(client, &txid_hex, &active_addr)
                .context("failed to locate vout for wallet address")?;
            let funding_amount = wallet.fetch_utxo_amount(funding_txid, None, funding_vout)?;

            let outpoint = OutPoint::new(funding_txid, funding_vout);
            wallet.register_utxo(outpoint, funding_amount)?;

            println!(
                "Mined and registered UTXO {}:{} with value {} sat for {} (txid {}).",
                outpoint.txid, outpoint.vout, funding_amount, active_addr, funding_txid
            );

            Ok(CommandOutcome::Continue)
        }
        "tx_status" => {
            let txid_str = parts.next().context("expected txid")?;
            let txid = Txid::from_str(txid_str).context("invalid txid format")?;
            match check_transaction_status(wallet, &txid) {
                Ok(info) => {
                    let mined = if info.confirmations > 0 { "yes" } else { "no" };
                    match (info.block_hash.as_deref(), info.block_height) {
                        (Some(hash), Some(height)) => println!(
                            "Tx {}: mined={} confirmations={} block_hash={} height={} total_outputs={:.8} BTC",
                            txid, mined, info.confirmations, hash, height, info.total_value_btc
                        ),
                        (Some(hash), None) => println!(
                            "Tx {}: mined={} confirmations={} block_hash={} total_outputs={:.8} BTC",
                            txid, mined, info.confirmations, hash, info.total_value_btc
                        ),
                        (None, _) => println!(
                            "Tx {}: mined={} confirmations={} total_outputs={:.8} BTC",
                            txid, mined, info.confirmations, info.total_value_btc
                        ),
                    };
                    Ok(CommandOutcome::Continue)
                }
                Err(err) => {
                    eprintln!("failed to query transaction status: {err}");
                    Ok(CommandOutcome::Continue)
                }
            }
        }
        "clear_db" => {
            require_operational_mode(mode, command)?;
            ensure!(
                wallet.network() == Network::Regtest,
                "clear_db is only available on regtest (current: {:?})",
                wallet.network()
            );
            wallet.clear_db()?;
            println!("Cleared UTXO database for regtest.");
            Ok(CommandOutcome::Continue)
        }
        "clear_funds" => {
            require_operational_mode(mode, command)?;
            if wallet.active_address().is_none() {
                bail!("import or switch to an address before clearing UTXOs");
            }
            let removed = wallet.clear_active_address_utxos()?;
            println!("Removed {} registered UTXO(s) for active address", removed);
            Ok(CommandOutcome::Continue)
        }
        "register_utxos_auto" => {
            require_operational_mode(mode, command)?;
            if wallet.network() != Network::Testnet {
                bail!(
                    "register_utxos_auto is only available on testnet (current: {:?})",
                    wallet.network()
                );
            }
            let active_address = wallet
                .active_address()
                .cloned()
                .context("import or switch to an address before auto-registering UTXOs")?;
            let active_address_str = active_address.to_string();
            println!(
                "Auto-registering testnet UTXOs for active wallet address: {}",
                active_address_str
            );

            let fetched = fetch_testnet_utxos(&active_address_str)?;
            let confirmed_utxos = filter_confirmed_utxos(fetched);

            let removed = wallet.clear_active_address_utxos()?;
            println!("Removed {} registered UTXO(s) for active address", removed);

            for utxo in &confirmed_utxos {
                let block_hash = utxo.status.block_hash.as_ref().context("missing block hash")?;
                let txid = Txid::from_str(&utxo.txid).context("invalid txid from Blockstream")?;
                // validate block_hash format before registering the UTXO
                bitcoincore_rpc::bitcoin::BlockHash::from_str(block_hash)
                    .context("invalid block hash from Blockstream")?;
                let outpoint = OutPoint::new(txid, utxo.vout);
                wallet.register_utxo(outpoint, utxo.value)?;
            }

            println!(
                "Registered {} confirmed UTXO(s) for {}",
                confirmed_utxos.len(),
                active_address_str
            );
            Ok(CommandOutcome::Continue)
        }
        "create_pegin_tx" => {
            require_operational_mode(mode, command)?;
            // Syntax: create_pegin_tx <stream_value> <packet_number> <dest_addr> <rsk_address> <enabler_script_pubkey>
            let stream_value_str = parts.next().context("expected stream value in satoshis")?;
            let stream_value: u64 =
                stream_value_str.parse().context("invalid stream value (satoshis)")?;

            let packet_number_str = parts.next().context("expected packet number")?;
            let packet_number: u64 = packet_number_str.parse().context("invalid packet number")?;

            let dest_addr = parts.next().context("expected destination address")?;
            let rsk_address = parts.next().context("expected RSK address (hex)")?;
            let enabler_script_pubkey =
                parts.next().context("expected enabler scriptPubKey (hex)")?;

            let created = wallet.create_pegin_transaction(
                stream_value,
                packet_number,
                dest_addr.to_string(),
                rsk_address.to_string(),
                enabler_script_pubkey.to_string(),
            )?;

            // Display transaction details
            let tx = &created.transaction;
            let txid = tx.compute_txid();
            let vsize = tx.vsize();
            let hex = serialize_hex(tx);

            println!("Pegin Transaction created:");
            println!("  txid={}", txid);
            println!("  vsize={}", vsize);
            println!("  fee={} sat", created.fee_sat);
            println!("  raw={}", hex);

            if let Some(change) = &created.change {
                println!(
                    "  change -> {} sat back to wallet (outpoint {}:{})",
                    change.value_sat, change.outpoint.txid, change.outpoint.vout
                );
            }

            // Broadcast if RPC is configured
            if wallet.rpc_client().is_some() {
                match wallet.broadcast_transaction(&created) {
                    Ok(txid) => println!("  Transaction broadcasted successfully: {}", txid),
                    Err(err) => eprintln!("  Failed to broadcast transaction: {err}"),
                }
            } else {
                println!("  RPC not configured; transaction hex printed only.");
            }

            Ok(CommandOutcome::Continue)
        }
        "list_pending" => {
            require_operational_mode(mode, command)?;
            let pending = wallet.pending_transaction_ids();
            if pending.is_empty() {
                println!("No pending transactions.");
            } else {
                println!("Pending transactions:");
                for txid in pending {
                    if let Some(created) = wallet.get_pending_transaction(&txid) {
                        println!(
                            "  {} - fee: {} sat, vsize: {} bytes",
                            txid,
                            created.fee_sat,
                            created.transaction.vsize()
                        );
                    } else {
                        println!("  {}", txid);
                    }
                }
            }
            Ok(CommandOutcome::Continue)
        }
        "replace_tx" => {
            require_operational_mode(mode, command)?;
            let txid_str = parts.next().context("expected txid")?;
            let new_fee_str = parts.next().context("expected new fee rate (sats/byte)")?;

            let txid = Txid::from_str(txid_str).context("invalid txid format")?;
            let new_sats_per_byte: u64 =
                new_fee_str.parse().context("invalid fee rate (must be positive integer)")?;

            println!(
                "Replacing transaction {} with fee rate {} sat/byte...",
                txid, new_sats_per_byte
            );

            let replacement = wallet.replace_transaction(txid, new_sats_per_byte)?;

            let new_txid = replacement.transaction.compute_txid();
            let vsize = replacement.transaction.vsize();
            let hex = serialize_hex(&replacement.transaction);

            println!("Replacement transaction created:");
            println!("  new_txid={}", new_txid);
            println!("  vsize={}", vsize);
            println!(
                "  fee={} sat (was {} sat)",
                replacement.fee_sat,
                wallet.get_pending_transaction(&txid).map(|t| t.fee_sat).unwrap_or(0)
            );
            println!("  raw={}", hex);

            if let Some(change) = &replacement.change {
                println!(
                    "  change -> {} sat back to wallet (outpoint {}:{})",
                    change.value_sat, change.outpoint.txid, change.outpoint.vout
                );
            }

            // broadcast the replacement
            if wallet.rpc_client().is_some() {
                match wallet.broadcast_transaction(&replacement) {
                    Ok(txid) => {
                        println!("  Replacement transaction broadcasted successfully: {}", txid)
                    }
                    Err(err) => eprintln!("  Failed to broadcast replacement transaction: {err}"),
                }
            } else {
                println!("  RPC not configured; replacement transaction hex printed only.");
            }

            Ok(CommandOutcome::Continue)
        }
        "confirm_tx" => {
            require_operational_mode(mode, command)?;
            let txid_str = parts.next().context("expected txid")?;
            let txid = Txid::from_str(txid_str).context("invalid txid format")?;

            wallet.confirm_transaction(txid)?;
            println!("Transaction {} confirmed and finalized.", txid);
            Ok(CommandOutcome::Continue)
        }
        "block_height" => {
            let Some(client) = wallet.rpc_client() else {
                println!("RPC not configured; block height query requires an RPC node.");
                return Ok(CommandOutcome::Continue);
            };
            let height: u64 = client.get_block_count().context("failed to query block height")?;
            println!("{}", height);
            Ok(CommandOutcome::Continue)
        }

        other => Err(anyhow!("unknown command '{other}'. Type 'help' for a list of commands.")),
    }
}

fn prompt_for(mode: &WalletMode, network_name: &str) -> String {
    let prompt_color = "\x1b[36m";
    let reset = "\x1b[0m";
    format!("{prompt_color}{mode}@{network_name}>{reset} ",)
}

fn print_active_address_utxos(wallet: &mut Wallet) -> Result<(), anyhow::Error> {
    let _: () = if let Some(address) = wallet.active_address() {
        let utxos = wallet.utxos_with_timestamps()?;
        println!("Registered UTXOs for {address}:");
        if utxos.is_empty() {
            println!("  (no registered UTXOs)");
        } else {
            print_utxos(&utxos);
        }
    } else {
        println!("No active address. Import or switch to an address to list funds.");
    };
    Ok(())
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

const SATS_PER_BTC: f64 = 100_000_000.0;
const MINER_FEE_BUFFER_SAT: u64 = 10_000;

fn sats_to_btc(amount_sat: u64) -> f64 {
    (amount_sat as f64) / SATS_PER_BTC
}

fn miner_trusted_balance_sat(client: &bitcoincore_rpc::Client) -> Result<u64> {
    let balances: Value =
        client.call("getbalances", &[]).context("failed to query wallet balances")?;
    parse_trusted_balance_sat(&balances)
}

fn parse_trusted_balance_sat(balances: &Value) -> Result<u64> {
    let trusted_btc = balances
        .get("mine")
        .and_then(|mine| mine.get("trusted"))
        .and_then(Value::as_f64)
        .context("bitcoind getbalances response missing mine.trusted")?;

    ensure!(trusted_btc >= 0.0, "bitcoind returned a negative trusted balance");

    Ok((trusted_btc * SATS_PER_BTC).round() as u64)
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

fn maybe_broadcast(wallet: &mut Wallet, txs: &[CreatedTransaction]) -> Result<()> {
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

fn print_help(sats_per_byte: u64, mode: &WalletMode) {
    let is_default = matches!(mode, WalletMode::Default);

    println!("Available commands:");
    println!("  help                                  - Show this message");
    println!("  exit | quit                           - Leave the wallet");
    println!(
        "  import_private_key <wif>              - Import compressed WIF for the current network kind"
    );
    println!(
        "  generate_address                      - Create a new P2WPKH key pair without switching active address"
    );
    println!("  list_addresses                        - Show imported wallet addresses");
    println!(
        "  list_funds [all]                      - List UTXOs for the active address or every address"
    );
    println!(
        "  tx_status <txid>                      - Query node for a tx: mined?, confirmations, block hash/height, total outputs"
    );
    println!(
        "  block_height                          - Get current blockchain height from RPC node"
    );

    if is_default {
        println!();
        println!("To access operational and signing commands, add your WIF as an environment");
        println!("variable and run with --mode user or --mode member.");
    } else {
        println!(
            "  switch_address <addr>                 - Make an imported address the active wallet address"
        );
        println!(
            "  register_utxo <txid> <block_hash> <vout> [sats] - Register a spendable P2WPKH UTXO"
        );
        println!(
            "  send_to_pubkey <hex_csv> <sats> [count]   - <hex_csv> is comma-separated compressed pubkeys (hex); create a single tx paying <sats> to each; repeat the whole tx by count (default 1)"
        );
        println!(
            "  send_to_address <addr_csv> <sats> [count] - <addr_csv> is comma-separated addresses (P2WPKH bech32 or P2PKH base58); create a single tx paying <sats> to each; repeat the whole tx by count (default 1)"
        );
        println!(
            "  mine_block                            - Regtest only: mine a single block via RPC"
        );
        println!(
            "  mine_utxo [sats]                      - Regtest only: mine and fund the active address with given amount (default 21000000 sat), then register the UTXO"
        );
        println!(
            "  clear_db                              - Regtest only: clear the UTXO database for the current network"
        );
        println!(
            "  clear_funds                           - Clear registered UTXOs for the active address"
        );
        println!(
            "  register_utxos_auto                   - Testnet only: refresh active-address UTXOs from Blockstream"
        );
        println!(
            "  create_pegin_tx <value> <packet> <addr> <rsk> <enabler>  - Create RSK pegin transaction"
        );
        println!();
        println!("RBF (Replace-By-Fee) commands:");
        println!(
            "  list_pending                          - Show all pending (unconfirmed) transactions"
        );
        println!(
            "  replace_tx <txid> <new_sats/byte>     - Replace pending transaction with higher fee"
        );
        println!(
            "  confirm_tx <txid>                     - Manually confirm a pending transaction (after on-chain confirmation)"
        );
    }

    println!();
    println!("Fees target {sats_per_byte} sat per virtual byte.");
}

fn format_timestamp(timestamp: u64) -> String {
    match i64::try_from(timestamp).ok().and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0)) {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => timestamp.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct BlockstreamUtxo {
    txid: String,
    vout: u32,
    value: u64,
    status: BlockstreamUtxoStatus,
}

#[derive(Debug, Deserialize)]
struct BlockstreamUtxoStatus {
    confirmed: bool,
    block_hash: Option<String>,
}

fn filter_confirmed_utxos(utxos: Vec<BlockstreamUtxo>) -> Vec<BlockstreamUtxo> {
    utxos
        .into_iter()
        .filter(|utxo| utxo.status.confirmed && utxo.status.block_hash.is_some())
        .collect()
}

fn fetch_testnet_utxos(address: &str) -> Result<Vec<BlockstreamUtxo>> {
    let url = format!("https://blockstream.info/testnet/api/address/{address}/utxo");
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;
    let response = client.get(&url).send().with_context(|| format!("failed to call {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_else(|_| String::from("<failed to read body>"));
        bail!("blockstream API returned {}: {}", status, body);
    }
    response.json::<Vec<BlockstreamUtxo>>().context("failed to parse blockstream UTXO response")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{BlockstreamUtxo, filter_confirmed_utxos, parse_trusted_balance_sat};

    #[test]
    fn parses_trusted_balance_from_getbalances_response() {
        let balances = json!({
            "mine": {
                "trusted": 0.2101,
                "untrusted_pending": 0.0,
                "immature": 50.0,
            }
        });

        assert_eq!(parse_trusted_balance_sat(&balances).unwrap(), 21_010_000);
    }

    #[test]
    fn errors_when_trusted_balance_is_missing() {
        let balances = json!({ "mine": { "immature": 50.0 } });

        assert!(parse_trusted_balance_sat(&balances).is_err());
    }

    fn utxo(txid: &str, confirmed: bool, block_hash: Option<&str>) -> BlockstreamUtxo {
        serde_json::from_value(json!({
            "txid": txid,
            "vout": 0,
            "value": 1000,
            "status": {
                "confirmed": confirmed,
                "block_hash": block_hash,
            },
        }))
        .expect("valid BlockstreamUtxo JSON")
    }

    #[test]
    fn filter_confirmed_utxos_keeps_confirmed_with_block_hash() {
        let kept = filter_confirmed_utxos(vec![utxo("aa", true, Some("bb"))]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].txid, "aa");
    }

    #[test]
    fn filter_confirmed_utxos_drops_unconfirmed() {
        let kept = filter_confirmed_utxos(vec![utxo("aa", false, Some("bb"))]);
        assert!(kept.is_empty());
    }

    #[test]
    fn filter_confirmed_utxos_drops_missing_block_hash() {
        let kept = filter_confirmed_utxos(vec![utxo("aa", true, None)]);
        assert!(kept.is_empty());
    }

    #[test]
    fn filter_confirmed_utxos_handles_empty_input() {
        assert!(filter_confirmed_utxos(Vec::new()).is_empty());
    }

    #[test]
    fn filter_confirmed_utxos_keeps_only_confirmed_with_block_hash() {
        let kept = filter_confirmed_utxos(vec![
            utxo("aa", true, Some("hash-a")),
            utxo("bb", false, Some("hash-b")),
            utxo("cc", true, None),
            utxo("dd", true, Some("hash-d")),
        ]);
        let txids: Vec<&str> = kept.iter().map(|u| u.txid.as_str()).collect();
        assert_eq!(txids, vec!["aa", "dd"]);
    }
}
