use crate::cache::Cache;
use crate::types::RskBlock;
use anyhow::Result;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const BLOCK_CACHE_SIZE: usize = 100;

pub struct CachedKeyValueStore {
    db: Storage,
    block_cache: Cache<RskBlock>,
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
            StoreKey::BestBlock => "meta/best_block_height".to_string(),
            StoreKey::BackSyncCheckpoint => "meta/tmp_back_sync_checkpoint".to_string(),
        }
    }
}

// TODO(iago) extract interface of this store

impl CachedKeyValueStore {
    pub fn new(path: &str) -> Result<Self> {
        let db = Storage::new_with_path(&PathBuf::from(format!("{}/.rootstock_monitor", path)))?;
        Ok(Self {
            db,
            block_cache: Cache::new(BLOCK_CACHE_SIZE),
        })
    }

    fn save_to_block_cache(&self, key: &str, value: &RskBlock) -> Result<Option<RskBlock>> {
        self.block_cache.insert(key, value)
    }

    fn get_from_block_cache(&self, key: &str) -> Result<Option<RskBlock>> {
        if let Some(cached_value) = self.block_cache.get(key)? {
            return Ok(Some(cached_value));
        }

        Ok(None)
    }

    pub fn get_best_block(&self) -> Result<Option<RskBlock>> {
        let key = &StoreKey::BestBlock.value();
        let cached_block = self.get_from_block_cache(key)?;
        Ok(cached_block.or(self.db.get(key)?))
    }

    pub fn set_best_block(&self, value: &RskBlock) -> Result<()> {
        let key = &StoreKey::BestBlock.value();
        self.save_to_block_cache(key, value)?;
        Ok(self.db.set(key, value)?)
    }

    pub fn get_back_sync_checkpoint(&self) -> Result<Option<RskBlock>> {
        let key = &StoreKey::BackSyncCheckpoint.value();
        let cached_block = self.get_from_block_cache(key)?;
        Ok(cached_block.or(self.db.get(key)?))
    }

    pub fn set_back_sync_checkpoint(&self, value: &RskBlock) -> Result<()> {
        let key = &StoreKey::BackSyncCheckpoint.value();
        self.save_to_block_cache(key, value)?;
        Ok(self.db.set(key, value)?)
    }

    pub fn reset_back_sync_checkpoint(&self) -> Result<()> {
        let key = &StoreKey::BackSyncCheckpoint.value();
        self.block_cache.remove(key)?;
        Ok(self.db.delete(key)?)
    }

    pub fn get_block_by_hash(&self, block_hash: &str) -> Result<Option<RskBlock>> {
        let key = &StoreKey::BlockByHash(block_hash.to_string()).value();
        let cached_block = self.get_from_block_cache(key)?;
        Ok(cached_block.or(self.db.get(key)?))
    }

    pub fn save_block(&self, value: &RskBlock) -> Result<()> {
        let key = &StoreKey::BlockByHash(value.hash().to_string()).value();
        self.save_to_block_cache(key, value)?;
        self.db.set(key, value)?;
        Ok(())
    }

    pub fn get_canonical_block(&self, block_height: u64) -> Result<Option<RskBlock>> {
        let key = &StoreKey::BlockByNumber(block_height).value();
        let cached_block_opt = self.get_from_block_cache(key)?;
        if cached_block_opt.is_some() {
            return Ok(cached_block_opt);
        }

        let block_hash: Option<String> = self.db.get(key)?;
        match block_hash {
            Some(block_hash) => Ok(self.get_block_by_hash(&block_hash)?),
            None => Ok(None),
        }
    }

    pub fn set_canonical_block(&self, block: &RskBlock) -> Result<()> {
        let key = &StoreKey::BlockByNumber(block.number()).value();
        self.save_to_block_cache(key, block)?;
        Ok(self.db.set(key, block.hash())?)
    }
}
