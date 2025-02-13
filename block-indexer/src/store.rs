use anyhow::Result;
use common::cache::{Cache, LruCache};
use common::types::RskBlock;
use std::env;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

pub trait BlockStore {
    fn get_best_block(&self) -> Result<Option<RskBlock>>;
    fn set_best_block(&self, value: &RskBlock) -> Result<()>;
    fn get_back_sync_checkpoint(&self) -> Result<Option<RskBlock>>;
    fn set_back_sync_checkpoint(&self, value: &RskBlock) -> Result<()>;
    fn reset_back_sync_checkpoint(&self) -> Result<()>;
    fn get_block_by_hash(&self, block_hash: &str) -> Result<Option<RskBlock>>;
    fn save_block(&self, value: &RskBlock) -> Result<()>;
    fn get_canonical_block(&self, block_height: u64) -> Result<Option<RskBlock>>;
    fn set_canonical_block(&self, block: &RskBlock) -> Result<()>;
}

pub struct CachedBlockStore<C: Cache<RskBlock>> {
    db: Storage,
    block_cache: C,
}

pub enum StoreKey {
    BlockByHash(String),
    BlockByNumber(u64),
    BestBlock,
    BackSyncCheckpoint,
}

impl StoreKey {
    pub fn value(&self) -> String {
        match self {
            StoreKey::BlockByHash(block_hash) => format!("block/hash/{}", block_hash),
            StoreKey::BlockByNumber(block_height) => format!("block/height/{}", block_height),
            // TODO(iago) check if the name is accurate
            StoreKey::BestBlock => "meta/best_block_height".to_string(),
            StoreKey::BackSyncCheckpoint => "meta/tmp_back_sync_checkpoint".to_string(),
        }
    }
}

impl<C: Cache<RskBlock>> CachedBlockStore<C> {
    fn save_to_cache(&self, key: &str, value: &RskBlock) -> Result<Option<RskBlock>> {
        self.block_cache.insert(key, value)
    }

    fn get_from_cache(&self, key: &str) -> Result<Option<RskBlock>> {
        if let Some(cached_value) = self.block_cache.get(key)? {
            return Ok(Some(cached_value));
        }

        Ok(None)
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

    fn get_best_block(&self) -> Result<Option<RskBlock>> {
        let key = StoreKey::BestBlock.value();

        if let Some(cached_block) = self.get_from_cache(&key)? {
            return Ok(Some(cached_block));
        }

        let db_block = self.get_from_db(&key)?;

        if let Some(ref block) = db_block {
            self.save_to_cache(&key, block)?;
        }

        Ok(db_block)
    }

    fn set_best_block(&self, value: &RskBlock) -> Result<()> {
        let key = &StoreKey::BestBlock.value();
        self.save_to_cache(key, value)?;
        Ok(self.set_on_db(key, value)?)
    }

    fn get_back_sync_checkpoint(&self) -> Result<Option<RskBlock>> {
        let key = &StoreKey::BackSyncCheckpoint.value();
        Ok(self.get_from_db(key)?)
    }

    fn set_back_sync_checkpoint(&self, value: &RskBlock) -> Result<()> {
        let key = &StoreKey::BackSyncCheckpoint.value();
        Ok(self.set_on_db(key, value)?)
    }

    fn reset_back_sync_checkpoint(&self) -> Result<()> {
        let key = &StoreKey::BackSyncCheckpoint.value();
        self.block_cache.remove(key)?;
        Ok(self.delete_from_db(key)?)
    }

    fn get_block_by_hash(&self, block_hash: &str) -> Result<Option<RskBlock>> {
        let key = StoreKey::BlockByHash(block_hash.to_string()).value();

        if let Some(cached_block) = self.get_from_cache(&key)? {
            return Ok(Some(cached_block));
        }

        let db_block = self.get_from_db(&key)?;

        if let Some(ref block) = db_block {
            self.save_to_cache(&key, block)?;
        }

        Ok(db_block)
    }

    fn save_block(&self, value: &RskBlock) -> Result<()> {
        let key = &StoreKey::BlockByHash(value.hash().to_string()).value();
        self.save_to_cache(key, value)?;
        self.set_on_db(key, value)?;
        Ok(())
    }

    fn get_canonical_block(&self, block_height: u64) -> Result<Option<RskBlock>> {
        let key = StoreKey::BlockByNumber(block_height).value();
        if let Some(cached_block) = self.get_from_cache(&key)? {
            return Ok(Some(cached_block));
        }

        let block_hash: String = match self.get_from_db(&key)? {
            Some(hash) => hash,
            None => return Ok(None),
        };

        let db_block = match self.get_block_by_hash(&block_hash)? {
            Some(block) => block,
            None => return Ok(None),
        };

        self.save_to_cache(&key, &db_block)?;

        Ok(Some(db_block))
    }

    fn set_canonical_block(&self, block: &RskBlock) -> Result<()> {
        let key = &StoreKey::BlockByNumber(block.number()).value();
        self.save_to_cache(key, block)?;
        Ok(self.set_on_db(key, &block.hash().to_string())?)
    }
}

impl CachedBlockStore<LruCache<RskBlock>> {
    pub fn new(path: &str) -> Result<Self> {
        let db = Storage::new_with_path(&PathBuf::from(path))?;
        let block_cache_size = env::var("BLOCK_CACHE_SIZE")
            .expect("BLOCK_CACHE_SIZE not set in env")
            .parse::<usize>()
            .expect("BLOCK_CACHE_SIZE in env must be a number");
        Ok(Self {
            db,
            block_cache: LruCache::new(block_cache_size),
        })
    }
}

impl<C: Cache<RskBlock>> BlockStore for CachedBlockStore<C> {
    fn get_best_block(&self) -> Result<Option<RskBlock>> {
        self.get_best_block()
    }

    fn set_best_block(&self, value: &RskBlock) -> Result<()> {
        self.set_best_block(value)
    }

    fn get_back_sync_checkpoint(&self) -> Result<Option<RskBlock>> {
        self.get_back_sync_checkpoint()
    }

    fn set_back_sync_checkpoint(&self, value: &RskBlock) -> Result<()> {
        self.set_back_sync_checkpoint(value)
    }

    fn reset_back_sync_checkpoint(&self) -> Result<()> {
        self.reset_back_sync_checkpoint()
    }

    fn get_block_by_hash(&self, block_hash: &str) -> Result<Option<RskBlock>> {
        self.get_block_by_hash(block_hash)
    }

    fn save_block(&self, value: &RskBlock) -> Result<()> {
        self.save_block(value)
    }

    fn get_canonical_block(&self, block_height: u64) -> Result<Option<RskBlock>> {
        self.get_canonical_block(block_height)
    }

    fn set_canonical_block(&self, block: &RskBlock) -> Result<()> {
        self.set_canonical_block(block)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::CachedBlockStore;
    use anyhow::Result;
    use common::{cache::LruCache, types::RskBlock};
    use serial_test::serial;
    use std::env;
    use tempfile::tempdir;

    // TODO: use real block data, also maybe use a fixture or some type of abstraction
    fn dummy_block(block_height: u64, block_hash: &str, parent_block_hash: &str) -> RskBlock {
        RskBlock::new(
            block_height,
            block_hash.to_string(),
            parent_block_hash.to_string(),
            Default::default(),
            0,
            "".to_string(),
            Default::default(),
        )
    }

    fn setup_env() {
        env::set_var("BLOCK_CACHE_SIZE", "20");
    }

    fn create_test_store() -> Result<CachedBlockStore<LruCache<RskBlock>>> {
        setup_env();
        let temp_dir = tempdir()?;
        let store_path = temp_dir.path().to_str().unwrap();
        let store = CachedBlockStore::new(store_path).expect("Failed to create CachedBlockStore");
        Ok(store)
    }

    #[test]
    #[serial]
    fn test_when_cache_size_exceeded_should_evict_old_entries() -> Result<()> {
        env::set_var("BLOCK_CACHE_SIZE", "2");
        let temp_dir = tempdir()?;
        let store_path = temp_dir.path().to_str().unwrap();
        let store = CachedBlockStore::new(store_path).expect("Failed to create CachedBlockStore");

        let block1 = dummy_block(1, "hash1", "hash0");
        let block2 = dummy_block(2, "hash2", "hash1");
        let block3 = dummy_block(3, "hash3", "hash2");
        store.save_block(&block1)?;
        store.save_block(&block2)?;
        store.save_block(&block3)?;
        let key1 = crate::store::StoreKey::BlockByHash("hash1".to_string()).value();
        let key2 = crate::store::StoreKey::BlockByHash("hash2".to_string()).value();
        let key3 = crate::store::StoreKey::BlockByHash("hash3".to_string()).value();
        let cached_block1 = store.block_cache.get(&key1)?;
        let cached_block2 = store.block_cache.get(&key2)?;
        let cached_block3 = store.block_cache.get(&key3)?;

        assert!(
            cached_block1.is_none(),
            "Block1 should have been evicted from the cache"
        );
        assert!(cached_block2.is_some(), "Block2 should be in the cache");
        assert!(cached_block3.is_some(), "Block3 should be in the cache");
        Ok(())
    }

    #[test]
    fn test_when_set_block_should_get_same_block() -> Result<()> {
        let store = create_test_store()?;
        let expected_block = dummy_block(100, "hash100", "hash99");

        store.set_best_block(&expected_block)?;
        let actual_block = store.get_best_block()?.unwrap();

        assert_eq!(expected_block, actual_block);
        Ok(())
    }

    #[test]
    fn test_when_save_block_should_get_by_hash_same_block() -> Result<()> {
        let store = create_test_store()?;
        let expected_block = dummy_block(100, "hash100", "hash99");

        store.save_block(&expected_block)?;
        let actual_block = store.get_block_by_hash("hash100")?.unwrap();

        assert_eq!(expected_block, actual_block);
        Ok(())
    }

    #[test]
    fn test_when_get_missing_hash_should_be_none() -> Result<()> {
        let store = create_test_store()?;
        let block = dummy_block(100, "hash100", "hash99");

        store.save_block(&block)?;
        let lookup = store.get_block_by_hash("hash999")?;

        assert!(
            lookup.is_none(),
            "Lookup for a non-existent block should return None"
        );
        Ok(())
    }

    #[test]
    fn test_when_set_checkpoint_should_get_same_checkpoint() -> Result<()> {
        let store = create_test_store()?;
        let expected_checkpoint = dummy_block(100, "hash100", "hash99");

        store.set_back_sync_checkpoint(&expected_checkpoint)?;
        let actual_checkpoint = store.get_back_sync_checkpoint()?.unwrap();

        assert_eq!(expected_checkpoint, actual_checkpoint);
        Ok(())
    }

    #[test]
    fn test_when_reset_checkpoint_should_be_none() -> Result<()> {
        let store = create_test_store()?;
        let checkpoint = dummy_block(100, "hash100", "hash99");

        store.set_back_sync_checkpoint(&checkpoint)?;
        store.reset_back_sync_checkpoint()?;
        let actual_checkpoint = store.get_back_sync_checkpoint()?;

        assert!(actual_checkpoint.is_none(), "Checkpoint should be reset");
        Ok(())
    }

    #[test]
    fn test_when_set_canonical_block_should_get_same_block() -> Result<()> {
        let store = create_test_store()?;
        let expected_canonical_block = dummy_block(100, "hash100", "hash99");

        store.set_canonical_block(&expected_canonical_block)?;
        let actual_canonical_block = store
            .get_canonical_block(expected_canonical_block.number())?
            .unwrap();

        assert_eq!(expected_canonical_block, actual_canonical_block);
        Ok(())
    }

    #[test]
    fn test_when_get_missing_canonical_block_should_return_none() -> Result<()> {
        let store = create_test_store()?;
        let canonical_block = dummy_block(100, "hash100", "hash99");

        store.set_canonical_block(&canonical_block)?;
        let missing_canonical_block = store.get_canonical_block(999)?;

        assert!(
            missing_canonical_block.is_none(),
            "Should return None for a missing canonical block"
        );
        Ok(())
    }

    #[test]
    fn test_when_get_block_by_hash_should_be_recached() -> Result<()> {
        let store = create_test_store()?;
        let expected_block = dummy_block(100, "hash100", "hash99");
        let cache_key = "block/hash/hash100";

        store.save_block(&expected_block)?;
        store.block_cache.remove(cache_key)?;
        let actual_block = store.get_block_by_hash("hash100")?.unwrap();
        let cached_block = store.block_cache.get(cache_key)?.unwrap();

        assert_eq!(expected_block, actual_block);
        assert_eq!(expected_block, cached_block);
        Ok(())
    }

    #[test]
    fn test_when_get_canonical_should_be_recached() -> Result<()> {
        let store = create_test_store()?;
        let expected_block = dummy_block(100, "hash100", "hash99");
        let cache_key = "block/height/100";

        store.set_canonical_block(&expected_block)?;
        store.save_block(&expected_block)?;
        store.block_cache.remove(cache_key)?;
        let actual_block = store.get_canonical_block(expected_block.number())?.unwrap();
        let cached_block = store.block_cache.get(cache_key)?.unwrap();

        assert_eq!(expected_block, actual_block);
        assert_eq!(expected_block, cached_block);
        Ok(())
    }

    #[test]
    fn test_when_delete_block_from_db_should_be_still_in_cache() -> Result<()> {
        let store = create_test_store()?;
        let expected_block = dummy_block(100, "hash100", "hash99");

        store.save_block(&expected_block)?;
        store.delete_from_db("block/hash/hash100")?;
        let actual_block = store.get_block_by_hash("hash100")?.unwrap();

        assert_eq!(expected_block, actual_block);
        Ok(())
    }
}

