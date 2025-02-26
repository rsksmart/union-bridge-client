use anyhow::Result;
use common::types::RskLog;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

pub trait LogStore {
    fn save_log(&self, value: &RskLog) -> Result<()>;
}

enum StoreKey {
    LogId(String, String, u64),
}

impl StoreKey {
    pub(super) fn value(&self) -> String {
        match self {
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

    fn set_on_db<T: serde::ser::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        Ok(self.db.set(key, value, None)?)
    }
}

impl LogStore for RawLogStore {
    fn save_log(&self, log: &RskLog) -> Result<()> {
        let key = StoreKey::LogId(
            log.info().address().to_string(),
            log.info().tx_hash().to_string(),
            log.info().log_index(),
        )
        .value();
        self.set_on_db(&key, log)?;
        Ok(())
    }
}
