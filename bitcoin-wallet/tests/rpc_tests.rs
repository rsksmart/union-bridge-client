use bitcoin::blockdata::transaction::Version;
use bitcoin::hashes::Hash;
use bitcoin::{Amount, ScriptBuf, Transaction, TxOut, Txid, absolute};
use bitcoincore_rpc::bitcoin::Txid as RpcTxid;
use ub_wallet::bitcoin::utils::{RawTxProvider, fetch_utxo_amount};

struct StubRpc {
    hex: String,
}

impl RawTxProvider for StubRpc {
    fn raw_transaction_hex(&self, _txid: &RpcTxid) -> bitcoincore_rpc::Result<String> {
        Ok(self.hex.clone())
    }
}

#[test]
fn fetch_utxo_amount_returns_value_from_hex() {
    let tx = Transaction {
        version: Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![],
        output: vec![TxOut {
            value: Amount::from_sat(21_000),
            script_pubkey: ScriptBuf::new(),
        }],
    };
    let stub = StubRpc {
        hex: bitcoin::consensus::encode::serialize_hex(&tx),
    };
    let txid = Txid::from_slice(&[0x33; 32]).expect("txid");
    let amount = fetch_utxo_amount(&stub, txid, 0).expect("amount");
    assert_eq!(amount, 21_000);
}

#[test]
fn fetch_utxo_amount_errors_when_vout_missing() {
    let tx = Transaction {
        version: Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![],
        output: vec![TxOut {
            value: Amount::from_sat(5_000),
            script_pubkey: ScriptBuf::new(),
        }],
    };
    let stub = StubRpc {
        hex: bitcoin::consensus::encode::serialize_hex(&tx),
    };
    let txid = Txid::from_slice(&[0x44; 32]).expect("txid");
    let err = fetch_utxo_amount(&stub, txid, 2).expect_err("should error");
    assert!(err.to_string().contains("vout"));
}
