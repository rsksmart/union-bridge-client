use anyhow::{anyhow, Result};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::RwLock;

pub struct Cache<V> {
    inner: RwLock<LruCache<String, V>>,
}

impl<V> Cache<V>
where
    V: Clone,
{
    pub fn new(max_size: usize) -> Self {
        Cache {
            // RwLock needed because LruCache requires mut access, which we don't want for our methods
            inner: RwLock::new(LruCache::new(NonZeroUsize::new(max_size).unwrap())),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<V>> {
        let mut cache = self
            .inner
            .write()
            .map_err(|e| anyhow!("Failed to acquire write lock on cache: {:?}", e))?;
        Ok(cache.get(key).cloned())
    }

    pub fn insert(&self, key: &str, value: &V) -> Result<Option<V>> {
        let mut cache = self
            .inner
            .write()
            .map_err(|e| anyhow!("Failed to acquire write lock on cache: {:?}", e))?;
        Ok(cache.put(key.to_string(), value.to_owned()))
    }

    pub fn remove(&self, key: &str) -> Result<Option<V>> {
        let mut write_guard = self
            .inner
            .write()
            .map_err(|e| anyhow!("Failed to acquire write lock on cache: {:?}", e))?;
        Ok(write_guard.pop(key))
    }
}
