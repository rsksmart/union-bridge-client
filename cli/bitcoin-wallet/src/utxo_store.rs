use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use bitcoin::Address;
use bitcoin::OutPoint;
use serde::{Deserialize, Serialize};
use storage_backend::storage::{KeyValueStore, Storage};
use storage_backend::storage_config::StorageConfig;

const UTXO_PREFIX: &str = "utxo/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UtxoState {
    Available,
    SpentUnconfirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUtxo {
    pub value_sat: u64,
    pub timestamp: u64,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default = "default_state")]
    pub state: UtxoState,
}

fn default_state() -> UtxoState {
    UtxoState::Available
}

pub struct UtxoStore {
    db: Storage,
}

impl UtxoStore {
    pub fn open(path: &Path) -> Result<Self> {
        let config = StorageConfig::new(path.to_string_lossy().to_string(), None);
        let db =
            Storage::new(&config).map_err(|e| anyhow!("failed to open storage backend: {e}"))?;
        Ok(Self { db })
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
            state: UtxoState::Available,
        };
        self.db.set(&key, &stored, None).map_err(|e| anyhow!("failed to write utxo: {e}"))
    }

    pub fn mark_spent_unconfirmed(&self, outpoint: &OutPoint) -> Result<()> {
        let key = utxo_key(outpoint);
        let mut stored: StoredUtxo = self
            .db
            .get(&key)
            .map_err(|e| anyhow!("failed to read utxo: {e}"))?
            .ok_or_else(|| anyhow!("utxo not found: {}", outpoint))?;
        stored.state = UtxoState::SpentUnconfirmed;
        self.db.set(&key, &stored, None).map_err(|e| anyhow!("failed to update utxo state: {e}"))
    }

    pub fn mark_available(&self, outpoint: &OutPoint) -> Result<()> {
        let key = utxo_key(outpoint);
        let mut stored: StoredUtxo = self
            .db
            .get(&key)
            .map_err(|e| anyhow!("failed to read utxo: {e}"))?
            .ok_or_else(|| anyhow!("utxo not found: {}", outpoint))?;
        stored.state = UtxoState::Available;
        self.db.set(&key, &stored, None).map_err(|e| anyhow!("failed to update utxo state: {e}"))
    }

    pub fn remove(&self, outpoint: &OutPoint) -> Result<()> {
        let key = utxo_key(outpoint);
        self.db.delete(&key).map_err(|e| anyhow!("failed to delete utxo: {e}"))
    }

    /// Iterates over all UTXOs in the store, yielding (OutPoint, StoredUtxo) pairs.
    /// Filters can be applied to the yielded items via the predicate.
    fn iter_utxos(
        &self,
        predicate: impl Fn(&StoredUtxo) -> bool,
    ) -> Result<Vec<(OutPoint, StoredUtxo)>> {
        let entries = self
            .db
            .partial_compare(UTXO_PREFIX)
            .map_err(|e| anyhow!("failed to iterate utxos: {e}"))?;
        let mut utxos = Vec::new();
        for (key, value) in entries.into_iter() {
            let stored: StoredUtxo = serde_json::from_str(&value)
                .map_err(|e| anyhow!("failed to deserialize utxo: {e}"))?;
            if predicate(&stored) {
                let outpoint = key_to_outpoint(&key)?;
                utxos.push((outpoint, stored));
            }
        }
        Ok(utxos)
    }

    pub fn load_all(&self) -> Result<Vec<(OutPoint, StoredUtxo)>> {
        self.iter_utxos(|_| true)
    }

    pub fn load_by_address(&self, address: &Address) -> Result<Vec<(OutPoint, StoredUtxo)>> {
        let address_str = address.to_string();
        // Note: Currently filters client-side by scanning all UTXOs. If the storage backend
        // supports address-specific prefix scanning (e.g., "utxo/{address}/"), we could optimize
        // this by storing UTXOs under an address-specific prefix structure.
        self.iter_utxos(|stored| {
            stored.address.as_deref().map_or(false, |addr| addr == address_str)
        })
    }

    pub fn contains(&self, outpoint: &OutPoint) -> Result<bool> {
        let key = utxo_key(outpoint);
        let exists: Option<StoredUtxo> =
            self.db.get(&key).map_err(|e| anyhow!("failed to read utxo: {e}"))?;
        Ok(exists.is_some())
    }

    pub fn clear(&self) -> Result<()> {
        let entries = self
            .db
            .partial_compare_keys(UTXO_PREFIX)
            .map_err(|e| anyhow!("failed to iterate utxos: {e}"))?;
        for key in entries.into_iter() {
            self.db.delete(&key).map_err(|e| anyhow!("failed to delete utxo: {e}"))?;
        }
        Ok(())
    }
}

fn utxo_key(outpoint: &OutPoint) -> String {
    format!("{}{:?}:{}", UTXO_PREFIX, outpoint.txid, outpoint.vout)
}

fn key_to_outpoint(key: &str) -> Result<OutPoint> {
    // Expected format: "utxo/<txid>:<vout>"
    let parts: Vec<&str> = key.split('/').collect();
    anyhow::ensure!(
        parts.len() == 2 && parts[0] == "utxo",
        "invalid UTXO key '{}': wrong prefix",
        key
    );
    let pair = parts[1];
    let mut iter = pair.rsplitn(2, ':');
    let vout_str = iter.next().context("missing vout in key")?;
    let txid_str = iter.next().context("missing txid in key")?;
    let txid = txid_str.parse().context("invalid txid in key")?;
    let vout: u32 = vout_str.parse().context("invalid vout in key")?;
    Ok(OutPoint::new(txid, vout))
}

fn current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
