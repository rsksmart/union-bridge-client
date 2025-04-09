use anyhow::{Result, anyhow};
use lru::LruCache as InnerLruCache;
use std::num::NonZeroUsize;
use std::sync::RwLock;

pub trait Cache<V> {
    fn get(&self, key: &str) -> Result<Option<V>>;
    fn insert(&self, key: &str, value: &V) -> Result<Option<V>>;
    fn remove(&self, key: &str) -> Result<Option<V>>;
}

pub struct LruCache<V> {
    inner: RwLock<InnerLruCache<String, V>>,
}

impl<V> LruCache<V>
where
    V: Clone,
{
    pub fn new(max_size: usize) -> Self {
        LruCache {
            // RwLock needed because LruCacheCrate requires mut access, which we don't want for our methods
            inner: RwLock::new(InnerLruCache::new(NonZeroUsize::new(max_size).unwrap())),
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

impl<V> Cache<V> for LruCache<V>
where
    V: Clone,
{
    fn get(&self, key: &str) -> Result<Option<V>> {
        self.get(key)
    }

    fn insert(&self, key: &str, value: &V) -> Result<Option<V>> {
        self.insert(key, value)
    }

    fn remove(&self, key: &str) -> Result<Option<V>> {
        self.remove(key)
    }
}
