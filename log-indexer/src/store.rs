use anyhow::Result;
use common::types::RskLog;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

pub trait LogStore {
    fn save_log(&self, value: &RskLog) -> Result<()>;
}

pub(super) enum StoreKey {
    LogId(String, String, u64),
}

impl StoreKey {
    pub(super) fn value(&self) -> String {
        match self {
            // TODO(iago) think of access patterns and prefix and indexing
            StoreKey::LogId(address, tx_hash, log_index) => {
                format!("logs/{}/{}/{}", address, tx_hash, log_index)
            }
        }
    }
}

pub struct RawLogStore {
    db: Storage,
}

impl RawLogStore {
    pub fn new(path: &str) -> Result<Self> {
        let db = Storage::new_with_path(&PathBuf::from(path))?;
        Ok(Self { db })
    }

    fn get_from_db<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self.db.get(key)?)
    }

    fn set_on_db<T: serde::ser::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        Ok(self.db.set(key, value, None)?)
    }

    fn delete_from_db(&self, key: &str) -> Result<()> {
        Ok(self.db.delete(key)?)
    }
}

impl LogStore for RawLogStore {
    fn save_log(&self, value: &RskLog) -> Result<()> {
        let key = StoreKey::LogId(
            value.address.to_string(),
            value.transaction_hash.to_string(),
            value.log_index,
        )
        .value();
        self.set_on_db(&key, value)?;
        Ok(())
    }
}
