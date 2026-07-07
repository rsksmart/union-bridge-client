use bitcoin::hashes::Hash;
use bitcoin::hashes::hex::FromHex;
use bitcoin::key::{CompressedPublicKey, PrivateKey};
use bitcoin::network::Network;
use bitcoin::secp256k1::{self, Secp256k1};
use bitcoin::{Address, Amount, ScriptBuf, Transaction, Txid};
use ub_wallet::bitcoin::utils::bytes_to_hex;
use ub_wallet::build_pegin::{BuildPeginRequest, UtxoInput, build_pegin};

#[test]
fn build_pegin_builds_signed_tx_with_change() {
    let secp = Secp256k1::new();

    // Funding wallet key -> regtest WIF.
    let wallet_secret = secp256k1::SecretKey::from_slice(&[1u8; 32]).expect("wallet secret");
    let wif = PrivateKey::new(wallet_secret, Network::Regtest).to_wif();

    // Destination (pegin) P2WPKH address from a second key.
    let dest_secret = secp256k1::SecretKey::from_slice(&[2u8; 32]).expect("dest secret");
    let dest_pk = PrivateKey::new(dest_secret, Network::Regtest).public_key(&secp);
    let dest_compressed = CompressedPublicKey::try_from(dest_pk).expect("compressed dest key");
    let pegin_address = Address::p2wpkh(&dest_compressed, Network::Regtest);

    // Enabler P2WPKH scriptPubKey (hex) from a third key.
    let enabler_secret = secp256k1::SecretKey::from_slice(&[3u8; 32]).expect("enabler secret");
    let enabler_pk = PrivateKey::new(enabler_secret, Network::Regtest).public_key(&secp);
    let enabler_wpkh = enabler_pk.wpubkey_hash().expect("compressed enabler key");
    let enabler_script = ScriptBuf::new_p2wpkh(&enabler_wpkh);
    let enabler_script_pubkey_hex = bytes_to_hex(enabler_script.as_bytes());

    // Single funding UTXO.
    let funding_txid = Txid::from_slice(&[0x11; 32]).expect("txid");
    let funding_value: u64 = 100_000_000;

    let stream_value_sat: u64 = 100_000;
    let sats_per_byte: u64 = 5;
    let enabler_amount_sat: u64 = 1_080;

    let req = BuildPeginRequest {
        wif,
        network: "regtest".to_string(),
        utxos: vec![UtxoInput {
            txid: funding_txid.to_string(),
            vout: 0,
            value_sat: funding_value,
        }],
        stream_value_sat,
        packet_number: 7,
        pegin_address: pegin_address.to_string(),
        rsk_address_hex: format!("0x{}", "11".repeat(20)),
        enabler_script_pubkey_hex,
        sats_per_byte: Some(sats_per_byte),
        enabler_amount_sat: Some(enabler_amount_sat),
    };

    let resp = build_pegin(req).expect("build pegin");

    // raw_tx_hex decodes back to a transaction with the reported txid.
    let raw = Vec::<u8>::from_hex(&resp.raw_tx_hex).expect("decode raw tx hex");
    let tx: Transaction = bitcoin::consensus::encode::deserialize(&raw).expect("deserialize tx");
    assert_eq!(tx.compute_txid().to_string(), resp.tx_id, "txid round-trips");

    // With ample funding, change is produced: 4 outputs.
    assert!(resp.change_value_sat > 0, "change produced");
    assert_eq!(tx.output.len(), 4, "stream + op_return + enabler + change");

    // output[0] = stream value to the pegin address.
    assert_eq!(tx.output[0].value, Amount::from_sat(stream_value_sat));
    assert_eq!(tx.output[0].script_pubkey, pegin_address.script_pubkey());

    // output[1] = value-0 OP_RETURN.
    assert_eq!(tx.output[1].value, Amount::from_sat(0));
    assert!(tx.output[1].script_pubkey.is_op_return());

    // output[2] = enabler amount to the enabler script.
    assert_eq!(tx.output[2].value, Amount::from_sat(enabler_amount_sat));
    assert_eq!(tx.output[2].script_pubkey, enabler_script);

    // output[3] = change.
    assert_eq!(tx.output[3].value, Amount::from_sat(resp.change_value_sat));

    // fee_sat == vsize * sats_per_byte.
    assert_eq!(resp.fee_sat, tx.vsize() as u64 * sats_per_byte);

    // spent_outpoints contains the funding outpoint.
    assert!(
        resp.spent_outpoints.iter().any(|o| o.txid == funding_txid.to_string() && o.vout == 0),
        "spent outpoints include the funding UTXO"
    );
}
