//! Programmatic usage example for the wallet library without spinning up the CLI loop.
//!
//! This example launches a regtest `bitcoind` instance (requires Docker), funds the wallet,
//! and demonstrates sending funds to both a bech32 address and directly to a compressed
//! public key, mirroring the `send_to_address` and `send_to_pubkey` CLI commands.

use std::str::FromStr;

use anyhow::{Context, Result};
use bitcoin::address::Address;
use bitcoin::key::{PrivateKey, PublicKey};
use bitcoin::network::Network;
use bitcoin::secp256k1::SecretKey;
use bitcoin::{OutPoint, Txid};
use bitcoincore_rpc::{Client, RpcApi};
use serde_json::{Value, json};
use ub_wallet::bitcoin::bitcoind::{Bitcoind, RpcConfig};
use ub_wallet::bitcoin::utils::{ensure_wallet, find_vout_for_address, wait_for_ready};
use ub_wallet::config::Config;
use ub_wallet::wallet::Wallet;

fn main() -> Result<()> {
    // 1. Launch regtest bitcoind so the wallet has an RPC endpoint to talk to.
    let bitcoind = start_bitcoind()?;
    let _bitcoind_guard = BitcoindGuard::new(&bitcoind);
    let wallet_client = ensure_wallet(&bitcoind).context("failed to init regtest wallet")?;

    // 2. Prepare configuration and temporary storage for the wallet library.
    let temp_dir = tempfile::tempdir()?;
    let wallet_root = temp_dir.path().to_path_buf();
    let rpc_cfg = bitcoind.rpc_config().clone();
    let wallet_rpc_url = format!("{}/wallet/{}", rpc_cfg.url, rpc_cfg.wallet);
    let wallet_config = Config {
        utxo_db_path: wallet_root,
        sats_per_byte: Some(5),
        network: Some(Network::Regtest),
        private_key_wif: None,
        rpc_url: Some(wallet_rpc_url),
        rpc_user: Some(rpc_cfg.username),
        rpc_password: Some(rpc_cfg.password),
        utxos: Vec::new(),
    };

    // 3. Create and exercise the wallet.
    let mut wallet = Wallet::from_config(&wallet_config)?;

    let funding_secret = SecretKey::from_slice(&[1u8; 32])?;
    let funding_key = PrivateKey::new(funding_secret, Network::Regtest);
    let funding_address = wallet.import_private_key(&funding_key.to_wif())?;
    println!("Loaded funding address: {}", funding_address);

    // 3.1 Fund the wallet with a mined UTXO.
    let funding_outpoint = mine_utxo(&wallet_client, &funding_address)?;
    let funding_amount = wallet.fetch_utxo_amount(funding_outpoint.txid, funding_outpoint.vout)?;
    wallet
        .register_utxo(funding_outpoint, funding_amount)
        .context("failed to register mined UTXO")?;
    println!(
        "Wallet funded with {} sat in tx {}:{}",
        funding_amount, funding_outpoint.txid, funding_outpoint.vout
    );

    // 3.2 Generate a recipient address
    let recipient_generated = wallet.generate_address()?;
    println!(
        "Generated recipient address: {}",
        recipient_generated.address
    );

    // 3.3 Send funds to the generated address
    let send_amount_sat = 60_000;
    let created_to_address = wallet.create_transactions(
        recipient_generated.address.script_pubkey(),
        send_amount_sat,
        1,
    )?;

    let address_txid = wallet.broadcast_transaction(&created_to_address[0])?;
    println!(
        "Broadcasted transaction {} sending {} sat to {}",
        address_txid, send_amount_sat, recipient_generated.address
    );

    // And verify that the broadcasted transaction is confirmed in a block.
    confirm_transaction(&wallet_client, &address_txid, "send_to_address output")?;

    // 3.4 Send funds directly to a public key
    let pubkey_generated = wallet.generate_address()?;
    let pubkey = PublicKey::from_str(&pubkey_generated.public_key_hex)
        .context("generated public key should be valid hex")?;

    let pubkey_amount_sat = 25_000;
    let pubkey_script = bitcoin::ScriptBuf::new_p2wpkh(
        &pubkey
            .wpubkey_hash()
            .map_err(|_| anyhow::anyhow!("public key must be compressed"))?,
    );

    let created_to_pubkey = wallet.create_transactions(pubkey_script, pubkey_amount_sat, 1)?;
    let pubkey_txid = wallet.broadcast_transaction(&created_to_pubkey[0])?;
    println!(
        "Broadcasted transaction {} sending {} sat to public key {}",
        pubkey_txid, pubkey_amount_sat, pubkey
    );

    // Verify that the broadcasted transaction is confirmed in a block.
    confirm_transaction(&wallet_client, &pubkey_txid, "send_to_pubkey output")?;

    println!("Remaining wallet UTXOs after sends:");
    for utxo in wallet.utxos() {
        println!("  {} ({} sat)", utxo.outpoint, utxo.value_sat);
    }

    println!(
        "Example finished. Initial funding UTXO: {}",
        funding_outpoint
    );

    Ok(())
}

struct BitcoindGuard<'a> {
    bitcoind: &'a Bitcoind,
}

impl<'a> BitcoindGuard<'a> {
    fn new(bitcoind: &'a Bitcoind) -> Self {
        Self { bitcoind }
    }
}

impl Drop for BitcoindGuard<'_> {
    fn drop(&mut self) {
        let _ = self.bitcoind.stop();
    }
}

fn start_bitcoind() -> Result<Bitcoind> {
    let bitcoind = Bitcoind::new(
        "ub-wallet-regtest-example",
        "ruimarinho/bitcoin-core",
        RpcConfig {
            username: "exampleuser".to_string(),
            password: "examplepass".to_string(),
            url: "http://127.0.0.1:18443".to_string(),
            wallet: "ub-wallet-example".to_string(),
            network: Network::Regtest,
        },
    );

    match bitcoind.start() {
        Ok(()) => {}
        Err(bollard::errors::Error::DockerResponseNotFoundError { message }) => {
            anyhow::bail!(
                "Docker daemon not available: {message}. Start Docker to run this example."
            );
        }
        Err(err) => anyhow::bail!("failed to start bitcoind container: {err}"),
    }

    wait_for_ready(&bitcoind)?;
    println!("bitcoind regtest container ready");
    Ok(bitcoind)
}

fn mine_utxo(client: &Client, target_address: &Address) -> Result<OutPoint> {
    let miner_address: String = client
        .call("getnewaddress", &[json!("miner"), json!("bech32")])
        .context("failed to obtain mining address")?;
    client
        .call::<Vec<String>>(
            "generatetoaddress",
            &[json!(101), json!(miner_address.clone())],
        )
        .context("failed to pre-mine regtest blocks")?;

    let send_amount_btc = 0.002_f64;
    let txid_hex: String = client
        .call(
            "sendtoaddress",
            &[json!(target_address.to_string()), json!(send_amount_btc)],
        )
        .context("failed to fund wallet")?;

    client
        .call::<Vec<String>>("generatetoaddress", &[json!(1), json!(miner_address)])
        .context("failed to confirm funding transaction")?;

    let funding_txid = Txid::from_str(&txid_hex).context("invalid txid returned by bitcoind")?;
    let funding_vout = find_vout_for_address(client, &txid_hex, target_address)
        .context("failed to locate vout for wallet address")?;

    Ok(OutPoint::new(funding_txid, funding_vout))
}

fn confirm_transaction(client: &Client, txid: &Txid, label: &str) -> Result<()> {
    let miner_address: String = client
        .call("getnewaddress", &[json!("miner"), json!("bech32")])
        .context("failed to obtain mining address for confirmation")?;
    let generated_blocks: Vec<String> = client
        .call("generatetoaddress", &[json!(1), json!(miner_address)])
        .context("failed to mine confirmation block")?;

    let verbose: Value = client
        .call("getrawtransaction", &[json!(txid.to_string()), json!(true)])
        .context("failed to fetch transaction details")?;

    let confirmations = verbose
        .get("confirmations")
        .and_then(|c| c.as_u64())
        .unwrap_or(0);

    let total_value_btc: f64 = verbose
        .get("vout")
        .and_then(|outs| outs.as_array())
        .map(|outs| {
            outs.iter()
                .filter_map(|out| out.get("value").and_then(|val| val.as_f64()))
                .sum()
        })
        .unwrap_or(0.0);

    let block_info = verbose
        .get("blockhash")
        .and_then(|h| h.as_str())
        .map(|hash| hash.to_string())
        .or_else(|| generated_blocks.into_iter().next());

    if let Some(block_hash) = block_info {
        let block_json: Value = client
            .call("getblock", &[json!(block_hash.clone())])
            .context("failed to fetch block information")?;
        let maybe_height = block_json.get("height").and_then(|h| h.as_i64());
        match maybe_height {
            Some(height) => println!(
                "Transaction {} ({}) confirmed in block {} at height {} with {} confirmations (total {:.8} BTC).",
                txid, label, block_hash, height, confirmations, total_value_btc
            ),
            None => println!(
                "Transaction {} ({}) confirmed in block {} with {} confirmations (total {:.8} BTC).",
                txid, label, block_hash, confirmations, total_value_btc
            ),
        }
    } else {
        println!(
            "Transaction {} ({}) has {} confirmations (block hash unavailable, total {:.8} BTC).",
            txid, label, confirmations, total_value_btc
        );
    }

    Ok(())
}
