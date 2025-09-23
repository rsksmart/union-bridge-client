use std::process;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use bitcoin::key::PrivateKey;
use bitcoin::network::Network;
use bitcoin::script::ScriptBuf;
use bitcoin::secp256k1::{self, Secp256k1};
use bitcoin::{OutPoint, Txid};
use bitcoincore_rpc::RpcApi;
use bollard::errors::Error as BollardError;
use serde_json::json;
use tempfile::tempdir;

use ub_wallet::bitcoin::bitcoind::{Bitcoind, RpcConfig};
use ub_wallet::bitcoin::utils::{ensure_wallet, find_vout_for_address, wait_for_ready};
use ub_wallet::wallet::Wallet;

const RPC_USER: &str = "walletcli";
const RPC_PASS: &str = "verysecret";
const BITCOIND_IMAGE: &str = "ruimarinho/bitcoin-core";
const RPC_URL: &str = "http://127.0.0.1:18443";

struct BitcoindGuard<'a> {
    node: &'a Bitcoind,
}

impl<'a> BitcoindGuard<'a> {
    fn new(node: &'a Bitcoind) -> Self {
        Self { node }
    }
}

impl Drop for BitcoindGuard<'_> {
    fn drop(&mut self) {
        let _ = self.node.stop();
    }
}

#[test]
#[ignore = "requires Docker daemon"]
fn wallet_end_to_end_over_regtest_rpc() -> Result<()> {
    let container_name = format!("ub-wallet-regtest-{}", process::id());
    let wallet_name = format!("ub-wallet-wallet-{}", process::id());

    let rpc_config = RpcConfig::new(
        Network::Regtest,
        RPC_URL.to_string(),
        RPC_USER.to_string(),
        RPC_PASS.to_string(),
        wallet_name,
    );

    let bitcoind = Bitcoind::new(&container_name, BITCOIND_IMAGE, rpc_config);

    match bitcoind.start() {
        Ok(()) => {}
        Err(BollardError::DockerResponseNotFoundError { message }) => {
            eprintln!("Skipping regtest RPC test: {message}");
            return Ok(());
        }
        Err(err) => return Err(anyhow!("failed to start bitcoind container: {err}")),
    }
    let _guard = BitcoindGuard::new(&bitcoind);

    wait_for_ready(&bitcoind)?;

    let wallet_client = ensure_wallet(&bitcoind)?;

    // Prepare miner wallet with spendable funds.
    let miner_address: String = wallet_client
        .call("getnewaddress", &[json!("miner"), json!("bech32")])
        .context("failed to obtain mining address")?;
    wallet_client
        .call::<Vec<String>>(
            "generatetoaddress",
            &[json!(101), json!(miner_address.clone())],
        )
        .context("failed to pre-mine regtest blocks")?;

    // Generate a new random key pair for the test wallet.
    let secp = Secp256k1::new();
    let mut rng = secp256k1::rand::thread_rng();
    let (secret_key, _) = secp.generate_keypair(&mut rng);
    let test_wallet_wif = PrivateKey {
        compressed: true,
        network: Network::Regtest.into(),
        inner: secret_key,
    }
    .to_wif();

    // Initialise CLI wallet backed by LevelDB store.
    let temp = tempdir().context("temp dir")?;
    let db_root = temp.path().join("utxo-db");
    let mut wallet = Wallet::new(db_root)?;
    let wallet_address = wallet.import_private_key(&test_wallet_wif)?;
    let wallet_address_str = wallet_address.to_string();

    // Send funds from miner wallet to the CLI wallet address.
    let send_amount_btc = 0.002_f64;
    let txid_hex: String = wallet_client
        .call(
            "sendtoaddress",
            &[json!(wallet_address_str.clone()), json!(send_amount_btc)],
        )
        .context("failed to send funds to wallet address")?;
    wallet_client
        .call::<Vec<String>>(
            "generatetoaddress",
            &[json!(1), json!(miner_address.clone())],
        )
        .context("failed to confirm funding transaction")?;

    wallet.set_rpc_client(wallet_client);

    let funding_txid = Txid::from_str(&txid_hex).context("invalid txid from sendtoaddress")?;
    let funding_vout = {
        let client = wallet
            .rpc_client()
            .context("wallet RPC client should be configured")?;
        find_vout_for_address(client, &txid_hex, &wallet_address)
            .context("failed to locate wallet output in funding transaction")?
    };
    let funding_amount = wallet.fetch_utxo_amount(funding_txid, funding_vout)?;
    wallet.register_utxo(OutPoint::new(funding_txid, funding_vout), funding_amount)?;

    assert_eq!(
        wallet.utxos().len(),
        1,
        "exactly one funding UTXO registered"
    );

    // Craft and broadcast a spend via the wallet.
    let target_script = recipient_script();
    let spend_amount_sat = 50_000_u64;
    let created = wallet
        .create_transactions(target_script, spend_amount_sat, 1)
        .context("wallet failed to build transaction")?;
    assert_eq!(created.len(), 1, "expected a single created transaction");

    let to_broadcast = &created[0];
    assert_eq!(
        to_broadcast.transaction.output[0].value.to_sat(),
        spend_amount_sat
    );

    let change = to_broadcast
        .change
        .as_ref()
        .context("wallet should return change for this spend")?;
    assert_eq!(
        wallet.utxos().len(),
        1,
        "change should replace spent UTXO in memory"
    );
    assert_eq!(wallet.utxos()[0].outpoint, change.outpoint);

    // Broadcast and confirm.
    let broadcast_txid = wallet
        .broadcast_transaction(to_broadcast)
        .context("failed to broadcast wallet transaction")?;
    assert_eq!(broadcast_txid, to_broadcast.transaction.compute_txid());

    wallet
        .rpc_client()
        .context("wallet RPC client should be configured")?
        .call::<Vec<String>>("generatetoaddress", &[json!(1), json!(miner_address)])
        .context("failed to confirm wallet transaction")?;

    let change_amount = wallet.fetch_utxo_amount(change.outpoint.txid, change.outpoint.vout)?;
    assert_eq!(change_amount, change.value_sat);

    Ok(())
}

fn recipient_script() -> ScriptBuf {
    let secp = Secp256k1::new();
    let sk = secp256k1::SecretKey::from_slice(&[2u8; 32]).expect("valid secret key");
    let pk = PrivateKey::new(sk, Network::Regtest).public_key(&secp);
    let wpkh = pk.wpubkey_hash().expect("compressed key");
    ScriptBuf::new_p2wpkh(&wpkh)
}
