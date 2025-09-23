use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::consensus::encode::deserialize;
use bitcoin::hashes::hex::FromHex;
use bitcoin::{Network, OutPoint, Transaction, Txid};
use bitcoincore_rpc::bitcoin::Txid as RpcTxid;
use bitcoincore_rpc::{Client, RpcApi};

use crate::bitcoin::bitcoind::{Bitcoind, RpcConfig};
use crate::wallet::Wallet;

pub trait RawTxProvider {
    fn raw_transaction_hex(&self, txid: &RpcTxid) -> bitcoincore_rpc::Result<String>;
}

impl RawTxProvider for Client {
    fn raw_transaction_hex(&self, txid: &RpcTxid) -> bitcoincore_rpc::Result<String> {
        self.get_raw_transaction_hex(txid, None)
    }
}

pub fn fetch_utxo_amount(provider: &impl RawTxProvider, txid: Txid, vout: u32) -> Result<u64> {
    let rpc_txid =
        RpcTxid::from_str(&txid.to_string()).context("failed to convert txid for RPC call")?;
    let tx_hex = provider
        .raw_transaction_hex(&rpc_txid)
        .context("failed to fetch transaction from RPC")?;
    let raw = Vec::<u8>::from_hex(&tx_hex).context("invalid transaction hex from RPC")?;
    let tx: Transaction = deserialize(&raw).context("failed to decode transaction")?;
    let tx_out = tx
        .output
        .get(vout as usize)
        .context("specified vout not found in transaction")?;
    Ok(tx_out.value.to_sat())
}

// Regtest only
pub fn send_test_funds(wallet: &mut Wallet) -> Result<()> {
    let private_key = wallet
        .private_key()
        .cloned()
        .context("wallet must have a loaded private key to send test funds")?;

    let wallet_network = wallet.network();
    anyhow::ensure!(
        wallet_network == Network::Regtest,
        "wallet network ({:?}) must be regtest to mine test funds",
        wallet_network
    );

    let Some(wallet_client) = wallet.rpc_client() else {
        println!("RPC not configured; skipping test fund generation.");
        return Ok(());
    };

    let miner_address: String = wallet_client
        .call(
            "getnewaddress",
            &[serde_json::json!("miner"), serde_json::json!("bech32")],
        )
        .context("failed to obtain mining address")?;
    wallet_client
        .call::<Vec<String>>(
            "generatetoaddress",
            &[
                serde_json::json!(101),
                serde_json::json!(miner_address.clone()),
            ],
        )
        .context("failed to pre-mine regtest blocks")?;

    let target_kind = bitcoin::NetworkKind::from(Network::Regtest);
    if private_key.network != target_kind {
        bail!(
            "private key network ({:?}) does not match current wallet network ({:?}).",
            private_key.network,
            wallet_network
        );
    }

    let public_key = private_key.public_key(&bitcoin::key::Secp256k1::new());
    let compressed = bitcoin::CompressedPublicKey::try_from(public_key)
        .map_err(|_| anyhow!("private key must correspond to a compressed public key"))?;
    let address = bitcoin::address::Address::p2wpkh(&compressed, Network::Regtest);

    let send_amount_btc = 0.002_f64;
    let txid_hex: String = wallet_client
        .call(
            "sendtoaddress",
            &[
                serde_json::json!(address.to_string().clone()),
                serde_json::json!(send_amount_btc),
            ],
        )
        .context("failed to send funds to wallet address")?;

    wallet_client
        .call::<Vec<String>>(
            "generatetoaddress",
            &[
                serde_json::json!(1),
                serde_json::json!(miner_address.clone()),
            ],
        )
        .context("failed to confirm funding transaction")?;

    let funding_txid = Txid::from_str(&txid_hex).context("invalid txid from sendtoaddress")?;
    let funding_vout = find_vout_for_address(wallet_client, &txid_hex, &address)
        .context("failed to locate wallet output in funding transaction")?;
    let funding_amount = wallet.fetch_utxo_amount(funding_txid, funding_vout)?;

    let outpoint = OutPoint::new(funding_txid, funding_vout);
    wallet
        .register_utxo(outpoint, funding_amount)
        .context("failed to store mined test funds")?;

    println!(
        "Sent {} BTC to address {} in txid {} (vout {}). Received {} sat.",
        send_amount_btc, address, funding_txid, funding_vout, funding_amount
    );

    Ok(())
}

// Local dockerized bitcoin client
pub fn start_client() -> Result<Client> {
    let bitcoin_client = Bitcoind::new(
        "bitcoin-regtest",
        "ruimarinho/bitcoin-core",
        RpcConfig {
            username: "foo".to_string(),
            password: "rpcpassword".to_string(),
            url: "http://127.0.0.1:18443".to_string(),
            wallet: "mywallet".to_string(),
            network: Network::Regtest,
        },
    );

    match bitcoin_client.start() {
        Ok(()) => {}
        Err(bollard::errors::Error::DockerResponseNotFoundError { message }) => {
            eprintln!("Skipping regtest RPC test: {message}");
            return Err(anyhow!("failed to start bitcoind container: {message}"));
        }
        Err(err) => return Err(anyhow!("failed to start bitcoind container: {err}")),
    }

    println!("Starting bitcoind in Regtest mode...");
    wait_for_ready(&bitcoin_client)?;
    println!("Bitcoind is ready");

    let wallet_client = ensure_wallet(&bitcoin_client)?;
    Ok(wallet_client)
}

pub fn wait_for_ready(bitcoind: &Bitcoind) -> Result<()> {
    let timeout = std::time::Duration::from_secs(20);
    let start = std::time::Instant::now();
    loop {
        let client = bitcoind
            .rpc_client()
            .context("create RPC client during readiness check")?;
        match client.get_block_count() {
            Ok(_) => return Ok(()),
            Err(err) if start.elapsed() > timeout => {
                return Err(anyhow!("bitcoind RPC not ready: {err}"));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
}

pub fn ensure_wallet(bitcoind: &Bitcoind) -> Result<Client> {
    let daemon_client = bitcoind.rpc_client().context("create daemon RPC client")?;
    let wallet_name = bitcoind.rpc_config().wallet.clone();
    let params = [
        serde_json::json!(wallet_name),
        serde_json::json!(false),
        serde_json::json!(false),
        serde_json::json!(""),
        serde_json::json!(false),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(false),
    ];
    match daemon_client.call::<serde_json::Value>("createwallet", &params) {
        Ok(_) => {}
        Err(err) if err.to_string().contains("already exists") => {}
        Err(err) => return Err(anyhow!("failed to create regtest wallet: {err}")),
    }

    bitcoind
        .wallet_client()
        .context("failed to create wallet RPC client")
}

pub fn find_vout_for_address(
    client: &Client,
    txid_hex: &str,
    address: &bitcoin::Address,
) -> Result<u32> {
    let verbose: serde_json::Value = client
        .call(
            "getrawtransaction",
            &[serde_json::json!(txid_hex), serde_json::json!(true)],
        )
        .context("failed to fetch funding transaction")?;
    let vouts = verbose
        .get("vout")
        .and_then(|v| v.as_array())
        .context("missing vout array in getrawtransaction output")?;

    let script_hex = bytes_to_hex(address.script_pubkey().as_bytes());
    for entry in vouts {
        if entry
            .get("scriptPubKey")
            .and_then(|spk| spk.get("hex"))
            .and_then(|h| h.as_str())
            == Some(script_hex.as_str())
        {
            let vout = entry
                .get("n")
                .and_then(|n| n.as_u64())
                .context("vout entry missing 'n'")?;
            return Ok(u32::try_from(vout).context("vout does not fit in u32")?);
        }
    }

    bail!("wallet output not found in transaction {txid_hex}");
}

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{:02x}", byte);
    }
    hex
}
