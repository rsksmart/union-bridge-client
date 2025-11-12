use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use bitcoin::consensus::deserialize;
use bitcoin::hashes::hex::FromHex;
use bitcoin::{Address, OutPoint, Transaction, Txid};
use serde::{Deserialize, Serialize};
use storage_backend::storage::{KeyValueStore, Storage};
use storage_backend::storage_config::StorageConfig;

use crate::wallet::{CreatedTransaction, Utxo};

const PENDING_TX_PREFIX: &str = "pending_tx/";

/// Serializable version of CreatedTransaction for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTransaction {
    transaction_hex: String,
    change_outpoint: Option<OutPoint>,
    change_value: Option<u64>,
    fee_sat: u64,
    spent_outpoints: Vec<OutPoint>,
    spent_values: Vec<u64>,
    change_address: String,
}

impl StoredTransaction {
    fn from_created(created: &CreatedTransaction) -> Result<Self> {
        let transaction_hex = bitcoin::consensus::encode::serialize_hex(&created.transaction);

        let (change_outpoint, change_value) = match &created.change {
            Some(utxo) => (Some(utxo.outpoint), Some(utxo.value_sat)),
            None => (None, None),
        };

        let spent_outpoints: Vec<OutPoint> =
            created.spent_utxos.iter().map(|u| u.outpoint).collect();
        let spent_values: Vec<u64> = created.spent_utxos.iter().map(|u| u.value_sat).collect();

        Ok(Self {
            transaction_hex,
            change_outpoint,
            change_value,
            fee_sat: created.fee_sat,
            spent_outpoints,
            spent_values,
            change_address: created.change_address.to_string(),
        })
    }

    fn to_created(&self, network: bitcoin::Network) -> Result<CreatedTransaction> {
        let tx_bytes =
            Vec::<u8>::from_hex(&self.transaction_hex).context("invalid transaction hex")?;
        let transaction: Transaction =
            deserialize(&tx_bytes).context("failed to decode transaction")?;

        let change = match (self.change_outpoint, self.change_value) {
            (Some(outpoint), Some(value_sat)) => Some(Utxo {
                outpoint,
                value_sat,
            }),
            _ => None,
        };

        let spent_utxos: Vec<Utxo> = self
            .spent_outpoints
            .iter()
            .zip(self.spent_values.iter())
            .map(|(outpoint, value_sat)| Utxo {
                outpoint: *outpoint,
                value_sat: *value_sat,
            })
            .collect();

        // reconstruct spent_indices - we don't actually need these for RBF,
        // but include empty vec for completeness
        let spent_indices = Vec::new();

        let change_address = Address::from_str(&self.change_address)
            .context("invalid change address")?
            .require_network(network)
            .context("address network mismatch")?;

        Ok(CreatedTransaction {
            transaction,
            change,
            fee_sat: self.fee_sat,
            spent_indices,
            spent_utxos,
            change_value: self.change_value.unwrap_or(0),
            change_address,
        })
    }
}

pub struct PendingTransactionStore {
    db: Storage,
}

impl PendingTransactionStore {
    pub fn open(path: &Path) -> Result<Self> {
        let config = StorageConfig::new(path.to_string_lossy().to_string(), None);
        let db = Storage::new(&config)
            .map_err(|e| anyhow!("failed to open pending transaction storage: {e}"))?;
        Ok(Self { db })
    }

    pub fn save(&self, txid: &Txid, created: &CreatedTransaction) -> Result<()> {
        let key = pending_tx_key(txid);
        let stored = StoredTransaction::from_created(created)?;
        self.db
            .set(&key, &stored, None)
            .map_err(|e| anyhow!("failed to save pending transaction: {e}"))
    }

    pub fn load(
        &self,
        txid: &Txid,
        network: bitcoin::Network,
    ) -> Result<Option<CreatedTransaction>> {
        let key = pending_tx_key(txid);
        let stored: Option<StoredTransaction> = self
            .db
            .get(&key)
            .map_err(|e| anyhow!("failed to read pending transaction: {e}"))?;

        match stored {
            Some(s) => Ok(Some(s.to_created(network)?)),
            None => Ok(None),
        }
    }

    pub fn load_all(&self, network: bitcoin::Network) -> Result<Vec<(Txid, CreatedTransaction)>> {
        let entries = self
            .db
            .partial_compare(PENDING_TX_PREFIX)
            .map_err(|e| anyhow!("failed to iterate pending transactions: {e}"))?;

        let mut transactions = Vec::new();
        for (key, value) in entries.into_iter() {
            let stored: StoredTransaction = serde_json::from_str(&value)
                .map_err(|e| anyhow!("failed to deserialize stored transaction: {e}"))?;
            let txid = key_to_txid(&key)?;
            let created = stored.to_created(network)?;
            transactions.push((txid, created));
        }
        Ok(transactions)
    }

    pub fn remove(&self, txid: &Txid) -> Result<()> {
        let key = pending_tx_key(txid);
        self.db
            .delete(&key)
            .map_err(|e| anyhow!("failed to delete pending transaction: {e}"))
    }

    pub fn clear(&self) -> Result<()> {
        let entries = self
            .db
            .partial_compare_keys(PENDING_TX_PREFIX)
            .map_err(|e| anyhow!("failed to iterate pending transactions: {e}"))?;
        for key in entries.into_iter() {
            self.db
                .delete(&key)
                .map_err(|e| anyhow!("failed to delete pending transaction: {e}"))?;
        }
        Ok(())
    }
}

fn pending_tx_key(txid: &Txid) -> String {
    format!("pending_tx/{}", txid)
}

fn key_to_txid(key: &str) -> Result<Txid> {
    // Expected format: "pending_tx/<txid>"
    let parts: Vec<&str> = key.split('/').collect();
    anyhow::ensure!(
        parts.len() == 2 && parts[0] == "pending_tx",
        "invalid pending transaction key '{}': wrong prefix",
        key
    );
    let txid_str = parts[1];
    let txid = txid_str.parse().context("invalid txid in key")?;
    Ok(txid)
}
