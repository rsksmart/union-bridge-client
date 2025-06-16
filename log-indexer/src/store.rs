use anyhow::Result;
use common::types::{Address, RskLog};
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait LogStore {
    fn save_log(&self, log: &RskLog) -> Result<()>;
    fn save_logs(&self, logs: &[RskLog]) -> Result<()>;
    fn get_sync_checkpoint(&self) -> Result<Option<RskLog>>;
    fn set_sync_checkpoint(&self, log: &RskLog) -> Result<()>;
}

enum StoreKey {
    LogId(Address, String, u64),
    LogSyncCheckpoint,
}

impl StoreKey {
    pub fn value(&self) -> String {
        match self {
            StoreKey::LogId(address, tx_hash, log_index) => {
                format!("logs/{}/{}/{}", address, tx_hash, log_index)
            }
            StoreKey::LogSyncCheckpoint => "meta/sync_checkpoint".to_string(),
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

    pub fn set<T: serde::ser::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        Ok(self.db.set(key, value, None)?)
    }

    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self.db.get(key)?)
    }

    /// Ideally, this method should be used only for testing purposes
    #[cfg(feature = "test-utils")]
    pub fn get_all_logs(&self) -> Result<Vec<RskLog>> {
        Ok(self
            .db
            .get_all()?
            .into_iter()
            .filter_map(|(key, log)| key.starts_with("logs/").then_some(log))
            .collect())
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
        // TODO(Jira): add bulk write operation to storage backend https://rsklabs.atlassian.net/browse/UB-113
        logs.iter().try_for_each(|log| self.save_log(log))
    }

    fn get_sync_checkpoint(&self) -> Result<Option<RskLog>> {
        let key = &StoreKey::LogSyncCheckpoint.value();

        Ok(self.get(key)?)
    }

    fn set_sync_checkpoint(&self, log: &RskLog) -> Result<()> {
        let key = &StoreKey::LogSyncCheckpoint.value();

        Ok(self.set(key, log)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use common::types::TxHash;
    use common::{
        test_utils::{rsk_log_generator::FakeLogGenerator, rsk_utils::generate_fake_address},
        types::{BlockHash, LogInfo},
    };
    use primitive_types::H256;
    use storage_backend::storage::KeyValueStore;
    use tempfile::tempdir;

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
            addr1.clone(),
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
            addr.clone(),
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
            addr.clone(),
            BlockHash::from(H256::random()),
            2.into(),
            TxHash::from(H256::random()),
            2,
            false,
        );
        let expected_log_info3 = LogInfo::new(
            addr.clone(),
            BlockHash::from(H256::random()),
            3.into(),
            TxHash::from(H256::random()),
            3,
            false,
        );
        let expected_log_info4 = LogInfo::new(
            addr.clone(),
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
            addr.clone(),
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
