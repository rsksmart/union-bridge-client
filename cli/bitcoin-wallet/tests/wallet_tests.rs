use bitcoin::hashes::Hash;
use bitcoin::key::PrivateKey;
use bitcoin::network::Network;
use bitcoin::script::ScriptBuf;
use bitcoin::secp256k1::{self, Secp256k1};
use bitcoin::{Amount, OutPoint, Txid};
use bitcoincore_rpc::jsonrpc;
use bitcoincore_rpc::jsonrpc::{Error, Request, Response};
use std::fmt::Formatter;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

use ub_wallet::cli::WalletMode;
use ub_wallet::utxo_store::UtxoStore;
use ub_wallet::wallet::Wallet;

fn recipient_script() -> ScriptBuf {
    let secp = Secp256k1::new();
    let sk = secp256k1::SecretKey::from_slice(&[2u8; 32]).expect("valid secret key");
    let pk = PrivateKey::new(sk, Network::Regtest).public_key(&secp);
    let wpkh = pk.wpubkey_hash().expect("compressed key");
    ScriptBuf::new_p2wpkh(&wpkh)
}

#[test]
fn create_transaction_does_not_consume_utxo_and_creates_change_until_broadcasted() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let mut wallet = Wallet::new(db_root, WalletMode::User).expect("wallet");
    let wallet_secret = secp256k1::SecretKey::from_slice(&[1u8; 32]).expect("wallet secret");
    let wallet_key = PrivateKey::new(wallet_secret, Network::Regtest);
    let wallet_wif = wallet_key.to_wif();
    wallet
        .import_private_key(&wallet_wif)
        .expect("import private key");

    set_fake_rpc_client(&mut wallet);

    let input_txid = Txid::from_slice(&[0x11; 32]).expect("txid");
    let utxo_amount = 50_000;
    wallet
        .register_utxo(OutPoint::new(input_txid, 0), utxo_amount)
        .expect("register utxo");

    let target_script = recipient_script();

    let tx_amount = 20_000;
    let txs = wallet
        .create_transactions(vec![target_script.clone()], 20_000, 1)
        .expect("create tx");

    assert_eq!(txs.len(), 1);
    let created = &txs[0];

    assert_eq!(created.transaction.output.len(), 2);
    assert_eq!(
        created.transaction.output[0].value,
        Amount::from_sat(tx_amount)
    );
    assert_eq!(created.transaction.output[0].script_pubkey, target_script);

    let expected_fee = (created.transaction.vsize() as u64) * wallet.sats_per_byte();
    assert_eq!(
        created.fee_sat, expected_fee,
        "fee should match rate * vsize"
    );

    let change_amount = created.transaction.output[1].value.to_sat();
    let expected_change = utxo_amount - tx_amount - expected_fee;
    assert_eq!(change_amount, expected_change);

    let change_entry = created.change.as_ref().expect("change output");
    assert_eq!(change_entry.value_sat, expected_change);

    // before broadcast, original utxo still present, change utxo not yet created
    let utxos = wallet.utxos();
    assert_eq!(utxos.len(), 1);
    assert_eq!(utxos[0].value_sat, utxo_amount);
    assert_eq!(utxos[0].outpoint.txid, input_txid);

    wallet
        .broadcast_transaction(&created)
        .expect("broadcast tx");

    // after broadcast, original utxo consumed, change utxo created
    let utxos = wallet.utxos();
    assert_eq!(utxos.len(), 1);
    assert_eq!(utxos[0].value_sat, expected_change);
    assert_eq!(utxos[0].outpoint.txid, change_entry.outpoint.txid);
}

#[test]
fn dust_change_is_added_to_fee() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let mut wallet = Wallet::new(db_root, WalletMode::User).expect("wallet");
    wallet.set_sats_per_byte(1); // keep target fee low to exercise dust logic

    set_fake_rpc_client(&mut wallet);

    let wallet_secret = secp256k1::SecretKey::from_slice(&[13u8; 32]).expect("wallet secret");
    let wallet_key = PrivateKey::new(wallet_secret, Network::Regtest);
    wallet
        .import_private_key(&wallet_key.to_wif())
        .expect("import private key");

    let input_txid = Txid::from_slice(&[0x21; 32]).expect("txid");
    let input_value = 50_000_u64;
    wallet
        .register_utxo(OutPoint::new(input_txid, 0), input_value)
        .expect("register utxo");

    let target_script = recipient_script();
    let send_value = 49_800_u64;

    let txs = wallet
        .create_transactions(vec![target_script], send_value, 1)
        .expect("create tx");

    assert_eq!(txs.len(), 1);
    let created = &txs[0];
    assert!(created.change.is_none(), "dust change should be skipped");
    assert_eq!(created.fee_sat, input_value - send_value);

    // before broadcast, original utxo still present, no change utxo created
    assert!(!wallet.utxos().is_empty(), "no change UTXO should remain");

    wallet
        .broadcast_transaction(&created)
        .expect("broadcast tx");

    // after broadcast, original utxo consumed, no change utxo created
    assert!(wallet.utxos().is_empty(), "no change UTXO should remain");
}

#[test]
fn switching_addresses_reloads_corresponding_utxos() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let mut wallet = Wallet::new(db_root, WalletMode::User).expect("wallet");

    let secret_one = secp256k1::SecretKey::from_slice(&[3u8; 32]).expect("secret one");
    let key_one = PrivateKey::new(secret_one, Network::Regtest);
    let addr_one = wallet
        .import_private_key(&key_one.to_wif())
        .expect("import first key");

    let txid_one = Txid::from_slice(&[0x33; 32]).expect("txid one");
    wallet
        .register_utxo(OutPoint::new(txid_one, 0), 42_000)
        .expect("register first utxo");
    assert_eq!(
        wallet.utxos().len(),
        1,
        "first address should have one utxo"
    );

    let secret_two = secp256k1::SecretKey::from_slice(&[4u8; 32]).expect("secret two");
    let key_two = PrivateKey::new(secret_two, Network::Regtest);
    let addr_two = wallet
        .import_private_key(&key_two.to_wif())
        .expect("import second key");

    assert!(
        wallet.utxos().is_empty(),
        "newly active second address should start without utxos"
    );

    let txid_two = Txid::from_slice(&[0x44; 32]).expect("txid two");
    wallet
        .register_utxo(OutPoint::new(txid_two, 1), 55_000)
        .expect("register second utxo");
    assert_eq!(
        wallet.utxos().len(),
        1,
        "second address should have one utxo"
    );

    wallet
        .switch_active_address(addr_one.clone())
        .expect("switch back to first address");
    assert_eq!(wallet.utxos().len(), 1, "first address utxo set restored");
    assert_eq!(wallet.utxos()[0].outpoint.txid, txid_one);

    wallet
        .switch_active_address(addr_two.clone())
        .expect("switch to second address");
    assert_eq!(wallet.utxos().len(), 1, "second address utxo set restored");
    assert_eq!(wallet.utxos()[0].outpoint.txid, txid_two);

    let addresses = wallet.imported_addresses();
    assert_eq!(addresses.len(), 2);
    assert!(addresses.contains(&addr_one.to_string()));
    assert!(addresses.contains(&addr_two.to_string()));
}

#[test]
fn switching_networks_with_mismatched_keys_succeeds() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let mut wallet = Wallet::new(db_root, WalletMode::User).expect("wallet");

    let secret = secp256k1::SecretKey::from_slice(&[12u8; 32]).expect("secret");
    let key = PrivateKey::new(secret, Network::Regtest);
    wallet
        .import_private_key(&key.to_wif())
        .expect("import regtest key");

    let changed = wallet
        .set_network(Network::Bitcoin)
        .expect("switch to bitcoin");
    assert!(changed, "network switch should report a change");
    assert!(wallet.imported_addresses().is_empty());
    assert!(wallet.active_address().is_none());
}

#[test]
fn utxos_with_timestamps_all_lists_active_first() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let mut wallet = Wallet::new(db_root, WalletMode::User).expect("wallet");

    let secret_one = secp256k1::SecretKey::from_slice(&[5u8; 32]).expect("secret one");
    let key_one = PrivateKey::new(secret_one, Network::Regtest);
    let addr_one = wallet
        .import_private_key(&key_one.to_wif())
        .expect("import first key");

    let txid_one = Txid::from_slice(&[0x55; 32]).expect("txid one");
    wallet
        .register_utxo(OutPoint::new(txid_one, 0), 21_000)
        .expect("register first utxo");

    let secret_two = secp256k1::SecretKey::from_slice(&[6u8; 32]).expect("secret two");
    let key_two = PrivateKey::new(secret_two, Network::Regtest);
    let addr_two = wallet
        .import_private_key(&key_two.to_wif())
        .expect("import second key");

    let txid_two = Txid::from_slice(&[0x66; 32]).expect("txid two");
    wallet
        .register_utxo(OutPoint::new(txid_two, 1), 34_000)
        .expect("register second utxo");

    let entries = wallet
        .utxos_with_timestamps_all()
        .expect("collect utxos across addresses");

    assert_eq!(entries.len(), 2, "both addresses should be reported");
    assert_eq!(entries[0].0, addr_two, "active address must appear first");
    assert_eq!(
        entries[0].1.len(),
        1,
        "active address should list its utxos"
    );
    assert_eq!(entries[0].1[0].0.outpoint, OutPoint::new(txid_two, 1));
    assert_eq!(entries[0].1[0].0.value_sat, 34_000);

    assert_eq!(
        entries[1].0, addr_one,
        "non-active address listed after active"
    );
    assert_eq!(entries[1].1.len(), 1);
    assert_eq!(entries[1].1[0].0.outpoint, OutPoint::new(txid_one, 0));
    assert_eq!(entries[1].1[0].0.value_sat, 21_000);

    let timestamps_are_nonzero = entries
        .iter()
        .flat_map(|(_, utxos)| utxos.iter().map(|(_, ts)| *ts))
        .all(|ts| ts > 0);
    assert!(timestamps_are_nonzero, "timestamps should be recorded");

    assert_eq!(wallet.active_address(), Some(&addr_two));
}

#[test]
fn utxos_with_timestamps_all_includes_store_only_addresses() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let address_string = {
        let mut wallet = Wallet::new(db_root.clone(), WalletMode::User).expect("wallet");

        let secret = secp256k1::SecretKey::from_slice(&[7u8; 32]).expect("secret");
        let key = PrivateKey::new(secret, Network::Regtest);
        let address = wallet
            .import_private_key(&key.to_wif())
            .expect("import key");

        let txid = Txid::from_slice(&[0x77; 32]).expect("txid");
        wallet
            .register_utxo(OutPoint::new(txid, 0), 45_000)
            .expect("register utxo");

        address.to_string()
    };

    let wallet = Wallet::new(db_root, WalletMode::User).expect("wallet reload");

    let entries = wallet
        .utxos_with_timestamps_all()
        .expect("load utxos across addresses");

    assert_eq!(entries.len(), 1, "address from store should be reported");
    assert_eq!(entries[0].0.to_string(), address_string);
    assert_eq!(entries[0].1.len(), 1, "stored utxo should be included");
    assert_eq!(entries[0].1[0].0.value_sat, 45_000);
}

#[test]
fn utxo_listings_are_sorted_by_timestamp() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");
    let mut setup_wallet = Wallet::new(db_root.clone(), WalletMode::User).expect("setup wallet");

    let secret = secp256k1::SecretKey::from_slice(&[8u8; 32]).expect("secret");
    let key = PrivateKey::new(secret, Network::Regtest);
    let wif = key.to_wif();
    let address = setup_wallet.import_private_key(&wif).expect("import key");
    drop(setup_wallet);

    // path structure is now: db_root/mode/network/utxo_db
    let store_path = db_root.join("user").join("regtest").join("utxo_db");
    let store = UtxoStore::open(&store_path).expect("reopen store for inserts");
    let txid_newer = Txid::from_slice(&[0x88; 32]).expect("txid newer");
    let txid_older = Txid::from_slice(&[0x99; 32]).expect("txid older");
    store
        .insert_with_timestamp(&OutPoint::new(txid_newer, 1), 50_000, 200, &address)
        .expect("insert newer utxo");
    store
        .insert_with_timestamp(&OutPoint::new(txid_older, 2), 25_000, 100, &address)
        .expect("insert older utxo");
    drop(store);

    let mut wallet = Wallet::new(db_root.clone(), WalletMode::User).expect("wallet");
    wallet
        .import_private_key(&wif)
        .expect("reimport key for listing");

    let listing = wallet.utxos_with_timestamps().expect("utxos with ts");
    assert_eq!(listing.len(), 2);
    assert_eq!(listing[0].1, 200);
    assert_eq!(listing[1].1, 100);

    let aggregated = wallet.utxos_with_timestamps_all().expect("aggregate utxos");
    assert_eq!(aggregated.len(), 1);
    let timestamps: Vec<u64> = aggregated[0].1.iter().map(|(_, ts)| *ts).collect();
    assert_eq!(timestamps, vec![200, 100]);

    drop(wallet);

    let wallet_reloaded = Wallet::new(db_root, WalletMode::User).expect("wallet reload");
    let aggregated_reloaded = wallet_reloaded
        .utxos_with_timestamps_all()
        .expect("aggregate utxos after reload");
    let timestamps_reloaded: Vec<u64> =
        aggregated_reloaded[0].1.iter().map(|(_, ts)| *ts).collect();
    assert_eq!(timestamps_reloaded, vec![200, 100]);
}

// Minimal fake JSON-RPC transport that returns a canned txid for sendrawtransaction
struct FakeTransport {
    // capture the last method called for debugging/validation if you want
    last_method: Arc<Mutex<Option<String>>>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            last_method: Arc::new(Mutex::new(None)),
        }
    }
}

impl jsonrpc::Transport for FakeTransport {
    fn send_request(&self, req: Request) -> Result<Response, Error> {
        let method = req.method.to_string();
        *self.last_method.lock().expect("lock last_method") = Some(method.clone());

        // Craft a canned txid string for sendrawtransaction
        let result = if method == "sendrawtransaction" {
            // 64 hex chars (little-endian txid string)
            let txid_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
            Some(
                serde_json::value::RawValue::from_string(format!("\"{}\"", txid_hex))
                    .expect("raw value"),
            )
        } else {
            // Default to null
            None
        };

        Ok(Response {
            result,
            error: None,
            id: req.id,
            jsonrpc: Some("2.0".to_string()),
        })
    }

    fn send_batch(&self, reqs: &[Request]) -> Result<Vec<Response>, Error> {
        reqs.iter().map(|r| self.send_request(r.clone())).collect()
    }

    fn fmt_target(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeTransport")
    }
}

fn set_fake_rpc_client(wallet: &mut Wallet) {
    // Install fake RPC client
    let transport = FakeTransport::new();
    let json_client = jsonrpc::Client::with_transport(transport);
    let client = bitcoincore_rpc::Client::from_jsonrpc(json_client);
    wallet.set_rpc_client(client);
}
