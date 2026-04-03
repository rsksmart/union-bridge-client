use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::consensus::encode::deserialize;
use bitcoin::hashes::hex::FromHex;
use bitcoin::{Transaction, Txid};
use bitcoincore_rpc::bitcoin::Txid as RpcTxid;
use bitcoincore_rpc::{Client, RpcApi};

use crate::bitcoin::bitcoind::Bitcoind;

pub trait RawTxProvider {
    fn raw_transaction_hex(
        &self,
        txid: &RpcTxid,
        block_hash: Option<&bitcoincore_rpc::bitcoin::BlockHash>,
    ) -> bitcoincore_rpc::Result<String>;
}

impl RawTxProvider for Client {
    fn raw_transaction_hex(
        &self,
        txid: &RpcTxid,
        block_hash: Option<&bitcoincore_rpc::bitcoin::BlockHash>,
    ) -> bitcoincore_rpc::Result<String> {
        self.get_raw_transaction_hex(txid, block_hash)
    }
}

pub fn fetch_utxo_amount(
    provider: &impl RawTxProvider,
    txid: Txid,
    block_hash: Option<&bitcoincore_rpc::bitcoin::BlockHash>,
    vout: u32,
) -> Result<u64> {
    let rpc_txid =
        RpcTxid::from_str(&txid.to_string()).context("failed to convert txid for RPC call")?;
    let tx_hex = provider
        .raw_transaction_hex(&rpc_txid, block_hash)
        .context("failed to fetch transaction from RPC")?;
    let raw = Vec::<u8>::from_hex(&tx_hex).context("invalid transaction hex from RPC")?;
    let tx: Transaction = deserialize(&raw).context("failed to decode transaction")?;
    let tx_out = tx.output.get(vout as usize).context("specified vout not found in transaction")?;
    Ok(tx_out.value.to_sat())
}

pub fn wait_for_ready(bitcoind: &Bitcoind) -> Result<()> {
    let timeout = std::time::Duration::from_secs(20);
    let start = std::time::Instant::now();
    loop {
        let client = bitcoind.rpc_client().context("create RPC client during readiness check")?;
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

    bitcoind.wallet_client().context("failed to create wallet RPC client")
}

pub fn find_vout_for_address(
    client: &Client,
    txid_hex: &str,
    address: &bitcoin::Address,
) -> Result<u32> {
    let verbose: serde_json::Value = client
        .call("getrawtransaction", &[serde_json::json!(txid_hex), serde_json::json!(true)])
        .context("failed to fetch funding transaction")?;
    let vouts = verbose
        .get("vout")
        .and_then(|v| v.as_array())
        .context("missing vout array in getrawtransaction output")?;

    let script_hex = bytes_to_hex(address.script_pubkey().as_bytes());
    for entry in vouts {
        if entry.get("scriptPubKey").and_then(|spk| spk.get("hex")).and_then(|h| h.as_str())
            == Some(script_hex.as_str())
        {
            let vout = entry.get("n").and_then(|n| n.as_u64()).context("vout entry missing 'n'")?;
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
