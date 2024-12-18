use crate::cache::Cache;
use crate::types::RskBlock;
use anyhow::Result;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

pub struct CachedKeyValueStore {
    db: Storage,
    block_cache: Cache<RskBlock>,
}

impl CachedKeyValueStore {
    pub fn new(path: &str) -> Result<Self> {
        let db = Storage::new_with_path(&PathBuf::from(format!("{}/.rootstock_monitor", path)))?;
        Ok(Self {
            db,
            block_cache: Cache::new(),
        })
    }

    fn save_to_cache<T: Clone>(&self, key: &str, value: &T, cache: &Cache<T>) -> Result<()>
    where
        T: serde::Serialize,
    {
        cache.insert(key, value)?;
        Ok(())
    }

    fn get_from_cache<T: Clone>(&self, key: &str, cache: &Cache<T>) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        if let Some(cached_value) = cache.get(key)? {
            return Ok(Some(cached_value));
        }

        Ok(None)
    }

    pub fn save_block(&self, key: &str, value: &RskBlock) -> Result<()> {
        self.save_to_cache(key, value, &self.block_cache)?;
        self.db.set(key, value)?;
        Ok(())
    }

    pub fn get_block(&self, key: &str) -> Result<Option<RskBlock>> {
        let cached_block = self.get_from_cache(key, &self.block_cache)?;
        if let Some(block) = cached_block {
            Ok(Some(block))
        } else {
            let block: Option<RskBlock> = self.db.get(key)?;
            Ok(block)
        }
    }
}
