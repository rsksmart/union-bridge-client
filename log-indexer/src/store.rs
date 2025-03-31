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

    /// Ideally, this method should be used only for testing purposes
    #[cfg(feature = "testing")]
    pub fn get(&self, key: String) -> Result<Option<RskLog>> {
        Ok(self.db.get(key)?)
    }

    /// Ideally, this method should be used only for testing purposes
    #[cfg(feature = "testing")]
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
            log.info().address().to_string(),
            log.info().tx_hash().to_string(),
            log.info().log_index(),
        )
        .value();
        self.set_on_db(&key, log)?;
        Ok(())
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::RawLogStore;
    use crate::store::{LogStore, StoreKey};
    use anyhow::Result;
    use common::types::{BlockHash, LogInfo};
    use primitive_types::H256;
    use storage_backend::storage::KeyValueStore;
    use tempfile::tempdir;
    use test_utils::rsk_log_generator::FakeLogGenerator;
    use test_utils::rsk_utils::generate_fake_address;

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
            H256::random().to_string(),
            1,
            false,
        );
        let expected_log = log_generator.generate_log(signature, expected_log_info);
        let log_key = StoreKey::LogId(
            expected_log.info().address().to_string(),
            expected_log.info().tx_hash().to_string(),
            expected_log.info().log_index(),
        )
        .value();

        store.save_log(&expected_log)?;
        let actual_log = store.get(log_key)?.unwrap();

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
            H256::random().to_string(),
            1,
            false,
        );
        let saved_log = log_generator.generate_log(signature, expected_log_info1);
        let log_key = StoreKey::LogId(
            saved_log.info().address().to_string(),
            saved_log.info().tx_hash().to_string(),
            saved_log.info().log_index(),
        )
        .value();
        let expected_log_info2 = LogInfo::new(
            addr.clone(),
            BlockHash::from(H256::random()),
            2.into(),
            H256::random().to_string(),
            2,
            false,
        );
        let expected_log_info3 = LogInfo::new(
            addr.clone(),
            BlockHash::from(H256::random()),
            3.into(),
            H256::random().to_string(),
            3,
            false,
        );
        let expected_log_info4 = LogInfo::new(
            addr.clone(),
            BlockHash::from(H256::random()),
            4.into(),
            H256::random().to_string(),
            4,
            false,
        );
        let different_log2 = log_generator.generate_log(signature, expected_log_info2);
        let different_log3 = log_generator.generate_log(signature, expected_log_info3);
        let different_log4 = log_generator.generate_log(signature, expected_log_info4);

        store.save_log(&saved_log)?;
        let actual_log = store.db.get(log_key)?.unwrap();

        assert_ne!(different_log2, actual_log);
        assert_ne!(different_log3, actual_log);
        assert_ne!(different_log4, actual_log);
        Ok(())
    }
}
