use anyhow::Result;
use common_core::types::{Address, RskLog};
use storage_backend::storage::{KeyValueStore, Storage};
use storage_backend::storage_config::StorageConfig;

const LOG_PREFIX: &str = "logs/";

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait LogStore {
    /// Save a log to storage
    ///
    /// # Errors
    ///
    /// Returns an error if the log cannot be serialized or saved to storage
    fn save_log(&self, log: &RskLog) -> Result<()>;
    /// Save multiple logs to storage
    ///
    /// # Errors
    ///
    /// Returns an error if any log cannot be serialized or saved to storage
    fn save_logs(&self, logs: &[RskLog]) -> Result<()>;
    /// Get the sync checkpoint log
    ///
    /// # Errors
    ///
    /// Returns an error if the checkpoint cannot be retrieved or deserialized from storage
    fn get_sync_checkpoint(&self) -> Result<Option<RskLog>>;
    /// Set the sync checkpoint log
    ///
    /// # Errors
    ///
    /// Returns an error if the checkpoint cannot be serialized or saved to storage
    fn set_sync_checkpoint(&self, log: &RskLog) -> Result<()>;
}

enum StoreKey {
    LogId(Address, String, u64),
    LogSyncCheckpoint,
}

impl StoreKey {
    pub(crate) fn value(&self) -> String {
        match self {
            StoreKey::LogId(address, tx_hash, log_index) => {
                format!("{LOG_PREFIX}{address}/{tx_hash}/{log_index}")
            }
            StoreKey::LogSyncCheckpoint => "meta/sync_checkpoint".to_string(),
        }
    }
}

pub struct RawLogStore {
    db: Storage,
}

impl RawLogStore {
    /// Create a new `RawLogStore`
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend cannot be initialized
    pub fn new(path: &str) -> Result<Self> {
        let config = StorageConfig::new(path.to_string(), None);
        let db = Storage::new(&config)?;
        Ok(Self { db })
    }

    /// Set a value in storage
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be serialized or saved to storage
    pub fn set<T: serde::ser::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        Ok(self.db.set(key, value, None)?)
    }

    /// Get a value from storage
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be retrieved or deserialized from storage
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self.db.get(key)?)
    }
}

impl LogStore for RawLogStore {
    fn save_log(&self, log: &RskLog) -> Result<()> {
        let key = StoreKey::LogId(
            log.info().address(),
            log.info().tx_hash().to_string(),
            log.info().log_index(),
        )
        .value();

        self.set(&key, log)?;

        Ok(())
    }

    fn save_logs(&self, logs: &[RskLog]) -> Result<()> {
        logs.iter().try_for_each(|log| self.save_log(log))
    }

    fn get_sync_checkpoint(&self) -> Result<Option<RskLog>> {
        let key = &StoreKey::LogSyncCheckpoint.value();

        self.get(key)
    }

    fn set_sync_checkpoint(&self, log: &RskLog) -> Result<()> {
        let key = &StoreKey::LogSyncCheckpoint.value();

        self.set(key, log)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use common_core::types::{BlockHash, LogInfo, TxHash};
    use common_dev::rsk_log_generator::FakeLogGenerator;
    use common_dev::rsk_utils::generate_fake_address;
    use primitive_types::H256;
    use storage_backend::storage::KeyValueStore;
    use tempfile::tempdir;

    use super::*;

    fn create_test_store() -> Result<RawLogStore> {
        let temp_dir = tempdir()?;
        let store_path = temp_dir.path().to_str().unwrap();
        let store = RawLogStore::new(store_path)?;
        Ok(store)
    }

    #[test]
    fn test_save_log() -> Result<()> {
        let store = create_test_store()?;
        let addr1 = generate_fake_address(1);
        let signature = "Transfer(address,address,uint256)";
        let log_generator: FakeLogGenerator = FakeLogGenerator::new();
        let expected_log_info = LogInfo::new(
            addr1,
            BlockHash::from(H256::random()),
            1.into(),
            TxHash::from(H256::random()),
            1,
            false,
        );
        let expected_log = log_generator.generate_log_with_info(signature, expected_log_info);
        let log_key = StoreKey::LogId(
            expected_log.info().address(),
            expected_log.info().tx_hash().to_string(),
            expected_log.info().log_index(),
        )
        .value();

        store.save_log(&expected_log)?;
        let actual_log = store.get(&log_key)?.unwrap();

        assert_eq!(expected_log, actual_log);
        Ok(())
    }

    #[test]
    fn test_save_log_no_different_log() -> Result<()> {
        let store = create_test_store()?;
        let addr = generate_fake_address(1);
        let signature = "Transfer(address,address,uint256)";
        let log_generator: FakeLogGenerator = FakeLogGenerator::new();
        let expected_log_info1 = LogInfo::new(
            addr,
            BlockHash::from(H256::random()),
            1.into(),
            TxHash::from(H256::random()),
            1,
            false,
        );
        let saved_log = log_generator.generate_log_with_info(signature, expected_log_info1);
        let log_key = StoreKey::LogId(
            saved_log.info().address(),
            saved_log.info().tx_hash().to_string(),
            saved_log.info().log_index(),
        )
        .value();
        let expected_log_info2 = LogInfo::new(
            addr,
            BlockHash::from(H256::random()),
            2.into(),
            TxHash::from(H256::random()),
            2,
            false,
        );
        let expected_log_info3 = LogInfo::new(
            addr,
            BlockHash::from(H256::random()),
            3.into(),
            TxHash::from(H256::random()),
            3,
            false,
        );
        let expected_log_info4 = LogInfo::new(
            addr,
            BlockHash::from(H256::random()),
            4.into(),
            TxHash::from(H256::random()),
            4,
            false,
        );
        let different_log2 = log_generator.generate_log_with_info(signature, expected_log_info2);
        let different_log3 = log_generator.generate_log_with_info(signature, expected_log_info3);
        let different_log4 = log_generator.generate_log_with_info(signature, expected_log_info4);

        store.save_log(&saved_log)?;
        let actual_log = store.db.get(log_key)?.unwrap();

        assert_ne!(different_log2, actual_log);
        assert_ne!(different_log3, actual_log);
        assert_ne!(different_log4, actual_log);
        Ok(())
    }

    #[test]
    fn test_when_set_checkpoint_should_get_same_checkpoint() -> Result<()> {
        let store = create_test_store()?;
        let addr = generate_fake_address(1);
        let signature = "Transfer(address,address,uint256)";
        let log_generator: FakeLogGenerator = FakeLogGenerator::new();
        let expected_log_info = LogInfo::new(
            addr,
            BlockHash::from(H256::random()),
            1.into(),
            TxHash::from(H256::random()),
            1,
            false,
        );
        let expected_checkpoint =
            log_generator.generate_log_with_info(signature, expected_log_info);

        store.set_sync_checkpoint(&expected_checkpoint)?;
        let actual_checkpoint = store.get_sync_checkpoint()?.unwrap();

        assert_eq!(expected_checkpoint, actual_checkpoint);
        Ok(())
    }
}
