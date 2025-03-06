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
    #[cfg(feature = "generate-mocks")]
    pub fn get(&self, key: String) -> Result<Option<RskLog>> {
        Ok(self.db.get(key)?)
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

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicBool, Arc};

    use super::RawLogStore;
    use crate::store::{LogStore, StoreKey};
    use anyhow::Result;
    use storage_backend::storage::KeyValueStore;
    use tempfile::tempdir;
    use test_utils::rsk_block_generator::FakeBlockGenerator;
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
        let block_generator: FakeBlockGenerator =
            FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
        let block = block_generator.generate_block(1.into());
        let log_generator: FakeLogGenerator =
            FakeLogGenerator::new("Transfer(address,address,uint256)");
        let expected_log = log_generator.generate_log(block, 1, addr1, 1);
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
        let addr1 = generate_fake_address(1);
        let addr2 = generate_fake_address(2);
        let block_generator: FakeBlockGenerator =
            FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
        let block = block_generator.generate_block(1.into());
        let log_generator: FakeLogGenerator =
            FakeLogGenerator::new("Transfer(address,address,uint256)");
        let saved_log = log_generator.generate_log(block.clone(), 1, addr1.clone(), 1);
        let log_key = StoreKey::LogId(
            saved_log.info().address().to_string(),
            saved_log.info().tx_hash().to_string(),
            saved_log.info().log_index(),
        )
        .value();
        let different_log = log_generator.generate_log(block.clone(), 1, addr2.clone(), 1);
        let different_log2 = log_generator.generate_log(block.clone(), 3, addr1.clone(), 2);
        let different_log3 = log_generator.generate_log(block.clone(), 2, addr1.clone(), 1);

        store.save_log(&saved_log)?;
        let actual_log = store.db.get(log_key)?.unwrap();

        assert_ne!(different_log, actual_log);
        assert_ne!(different_log2, actual_log);
        assert_ne!(different_log3, actual_log);
        Ok(())
    }
}
