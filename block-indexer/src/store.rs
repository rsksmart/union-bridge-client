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
