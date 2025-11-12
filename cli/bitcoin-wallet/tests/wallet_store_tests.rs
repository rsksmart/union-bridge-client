use bitcoin::hashes::Hash;
use bitcoin::key::PrivateKey;
use bitcoin::network::Network;
use bitcoin::secp256k1;
use bitcoin::secp256k1::SecretKey;
use bitcoin::{OutPoint, Txid};
use tempfile::tempdir;

use ub_wallet::cli::WalletMode;
use ub_wallet::utxo_store::UtxoStore;
use ub_wallet::wallet::Wallet;

#[test]
fn utxos_persist_across_wallet_instances() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");

    // First wallet registers a UTXO.
    {
        let mut wallet = Wallet::new(db_root.clone(), WalletMode::User).expect("wallet init");

        let secret = secp256k1::SecretKey::from_slice(&[42u8; 32]).expect("secret");
        let key = PrivateKey::new(secret, Network::Regtest);
        wallet
            .import_private_key(&key.to_wif())
            .expect("import key");

        let txid = Txid::from_slice(&[0x22; 32]).expect("txid");
        wallet
            .register_utxo(OutPoint::new(txid, 1), 75_000)
            .expect("register utxo");
    }

    // New wallet instance using the same store should see the previously registered UTXO.
    let mut wallet = Wallet::new(db_root, WalletMode::User).expect("wallet init");
    let secret = secp256k1::SecretKey::from_slice(&[42u8; 32]).expect("secret");
    let key = PrivateKey::new(secret, Network::Regtest);
    wallet
        .import_private_key(&key.to_wif())
        .expect("re-import key");
    let utxos = wallet.utxos();
    assert_eq!(utxos.len(), 1);
    assert_eq!(utxos[0].value_sat, 75_000);
    assert_eq!(utxos[0].outpoint.vout, 1);
}

#[test]
fn load_by_address_returns_only_matching_entries() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");

    let mut wallet = Wallet::new(db_root.clone(), WalletMode::User).expect("wallet init");

    let secret_one = secp256k1::SecretKey::from_slice(&[10u8; 32]).expect("secret one");
    let key_one = PrivateKey::new(secret_one, Network::Regtest);
    let addr_one = wallet
        .import_private_key(&key_one.to_wif())
        .expect("import first key");
    let txid_one = Txid::from_slice(&[0x55; 32]).expect("txid one");
    wallet
        .register_utxo(OutPoint::new(txid_one, 0), 11_000)
        .expect("register first utxo");

    let secret_two = secp256k1::SecretKey::from_slice(&[11u8; 32]).expect("secret two");
    let key_two = PrivateKey::new(secret_two, Network::Regtest);
    let addr_two = wallet
        .import_private_key(&key_two.to_wif())
        .expect("import second key");
    let txid_two = Txid::from_slice(&[0x66; 32]).expect("txid two");
    wallet
        .register_utxo(OutPoint::new(txid_two, 1), 22_000)
        .expect("register second utxo");

    drop(wallet);

    // path structure is now: db_root/mode/network/utxo_db
    let store_path = db_root.join("user").join("regtest").join("utxo_db");
    let store = UtxoStore::open(&store_path).expect("re-open store");
    let addr_one_str = addr_one.to_string();
    let entries_one = store
        .load_by_address(&addr_one)
        .expect("load entries for first address");
    assert_eq!(entries_one.len(), 1);
    assert_eq!(entries_one[0].0.txid, txid_one);
    assert_eq!(entries_one[0].1.value_sat, 11_000);
    assert_eq!(
        entries_one[0].1.address.as_deref(),
        Some(addr_one_str.as_str())
    );

    let addr_two_str = addr_two.to_string();
    let entries_two = store
        .load_by_address(&addr_two)
        .expect("load entries for second address");
    assert_eq!(entries_two.len(), 1);
    assert_eq!(entries_two[0].0.txid, txid_two);
    assert_eq!(entries_two[0].1.value_sat, 22_000);
    assert_eq!(
        entries_two[0].1.address.as_deref(),
        Some(addr_two_str.as_str())
    );

    let all = store.load_all().expect("load all utxos");
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|(_, stored)| stored.address.is_some()));
}

#[test]
fn switching_networks_preserves_utxos_per_network() {
    let temp = tempdir().expect("temp dir");
    let db_root = temp.path().join("utxo-db");

    let mut wallet = Wallet::new(db_root.clone(), WalletMode::User).expect("wallet init");
    let secret = SecretKey::from_slice(&[12u8; 32]).expect("secret");
    let key = PrivateKey::new(secret, Network::Regtest);
    let address = wallet
        .import_private_key(&key.to_wif())
        .expect("import key");

    let txid = Txid::from_slice(&[0x12; 32]).expect("txid");
    wallet
        .register_utxo(OutPoint::new(txid, 0), 33_000)
        .expect("register utxo");

    wallet
        .set_network(Network::Testnet)
        .expect("switch to testnet");

    wallet
        .set_network(Network::Regtest)
        .expect("switch back to regtest");

    let reloaded_addr = wallet
        .import_private_key(&key.to_wif())
        .expect("re-import key");
    assert_eq!(reloaded_addr, address);

    let utxos = wallet.utxos();
    assert_eq!(utxos.len(), 1);
    assert_eq!(utxos[0].outpoint.txid, txid);
    assert_eq!(utxos[0].value_sat, 33_000);
}
