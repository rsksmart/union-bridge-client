use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct Cache<V> {
    inner: Arc<RwLock<HashMap<String, V>>>,
}

impl<V> Cache<V>
where
    V: Clone,
{
    pub fn new() -> Self {
        Cache {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<V>> {
        let read_guard = self
            .inner
            .read()
            .map_err(|e| anyhow!("Failed to acquire read lock on cache: {:?}", e))?;
        Ok(read_guard.get(key).cloned())
    }

    pub fn insert(&self, key: &str, value: &V) -> Result<()> {
        let mut write_guard = self
            .inner
            .write()
            .map_err(|e| anyhow!("Failed to acquire write lock on cache: {:?}", e))?;
        write_guard.insert(key.to_string(), value.to_owned());
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<Option<V>> {
        let mut write_guard = self
            .inner
            .write()
            .map_err(|e| anyhow!("Failed to acquire write lock on cache: {:?}", e))?;
        Ok(write_guard.remove(key))
    }
}
