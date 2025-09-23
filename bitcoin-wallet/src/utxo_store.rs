use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use bitcoin::Address;
use bitcoin::OutPoint;
use bitcoin::hashes::Hash;
use rusty_leveldb::{DB, LdbIterator, Options};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUtxo {
    pub value_sat: u64,
    pub timestamp: u64,
    #[serde(default)]
    pub address: Option<String>,
}

pub struct UtxoStore {
    db: Mutex<DB>,
}

impl UtxoStore {
    pub fn open(path: &Path) -> Result<Self> {
        let mut options = Options::default();
        options.create_if_missing = true;
        let db = DB::open(path, options).map_err(|e| anyhow!("failed to open LevelDB: {e}"))?;
        Ok(Self { db: Mutex::new(db) })
    }

    pub fn insert(&self, outpoint: &OutPoint, value_sat: u64, address: &Address) -> Result<()> {
        self.insert_with_timestamp(outpoint, value_sat, current_timestamp(), address)
    }

    pub fn insert_with_timestamp(
        &self,
        outpoint: &OutPoint,
        value_sat: u64,
        timestamp: u64,
        address: &Address,
    ) -> Result<()> {
        let key = utxo_key(outpoint);
        let stored = StoredUtxo {
            value_sat,
            timestamp,
            address: Some(address.to_string()),
        };
        let bytes = serde_json::to_vec(&stored).context("serialize utxo")?;
        let mut db = self.lock_db()?;
        db.put(&key, &bytes)
            .map_err(|e| anyhow!("failed to write utxo: {e}"))?;
        db.flush()
            .map_err(|e| anyhow!("failed to flush utxo store: {e}"))
    }

    pub fn remove(&self, outpoint: &OutPoint) -> Result<()> {
        let key = utxo_key(outpoint);
        let mut db = self.lock_db()?;
        db.delete(&key)
            .map_err(|e| anyhow!("failed to delete utxo: {e}"))?;
        db.flush()
            .map_err(|e| anyhow!("failed to flush utxo store: {e}"))
    }

    pub fn load_all(&self) -> Result<Vec<(OutPoint, StoredUtxo)>> {
        let mut db = self.lock_db()?;
        let mut iter = db
            .new_iter()
            .map_err(|e| anyhow!("failed to create iterator: {e}"))?;
        let mut utxos = Vec::new();
        while let Some((key, value)) = iter.next() {
            let outpoint = key_to_outpoint(&key)?;
            let stored: StoredUtxo = serde_json::from_slice(&value).context("parse stored utxo")?;
            utxos.push((outpoint, stored));
        }
        Ok(utxos)
    }

    pub fn load_by_address(&self, address: &Address) -> Result<Vec<(OutPoint, StoredUtxo)>> {
        let address_str = address.to_string();
        let mut db = self.lock_db()?;
        let mut iter = db
            .new_iter()
            .map_err(|e| anyhow!("failed to create iterator: {e}"))?;
        let mut utxos = Vec::new();
        while let Some((key, value)) = iter.next() {
            let stored: StoredUtxo = serde_json::from_slice(&value).context("parse stored utxo")?;
            if stored
                .address
                .as_deref()
                .map_or(false, |addr| addr == address_str)
            {
                let outpoint = key_to_outpoint(&key)?;
                utxos.push((outpoint, stored));
            }
        }
        Ok(utxos)
    }

    pub fn contains(&self, outpoint: &OutPoint) -> Result<bool> {
        let key = utxo_key(outpoint);
        let mut db = self.lock_db()?;
        Ok(db.get(&key).is_some())
    }

    pub fn clear(&self) -> Result<()> {
        let mut db = self.lock_db()?;
        let mut iter = db
            .new_iter()
            .map_err(|e| anyhow!("failed to create iterator: {e}"))?;
        let mut keys = Vec::new();
        while let Some((key, _)) = iter.next() {
            keys.push(key);
        }
        for key in keys {
            db.delete(&key)
                .map_err(|e| anyhow!("failed to delete utxo: {e}"))?;
        }
        db.flush()
            .map_err(|e| anyhow!("failed to flush utxo store: {e}"))
    }

    fn lock_db(&self) -> Result<MutexGuard<'_, DB>> {
        self.db
            .lock()
            .map_err(|_| anyhow!("UTXO database mutex poisoned"))
    }
}

fn utxo_key(outpoint: &OutPoint) -> Vec<u8> {
    let mut key = Vec::with_capacity(36);
    key.extend_from_slice(&outpoint.txid.to_byte_array());
    key.extend_from_slice(&outpoint.vout.to_le_bytes());
    key
}

fn key_to_outpoint(key: &[u8]) -> Result<OutPoint> {
    anyhow::ensure!(key.len() == 36, "invalid UTXO key len {}", key.len());
    let mut txid_bytes = [0u8; 32];
    txid_bytes.copy_from_slice(&key[0..32]);
    let txid = bitcoin::Txid::from_byte_array(txid_bytes);
    let mut vout_bytes = [0u8; 4];
    vout_bytes.copy_from_slice(&key[32..36]);
    let vout = u32::from_le_bytes(vout_bytes);
    Ok(OutPoint::new(txid, vout))
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
