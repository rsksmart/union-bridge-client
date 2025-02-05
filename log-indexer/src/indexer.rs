use crate::store::LogStore;
use anyhow::Result;
use definitions::rsk_indexer::RskIndexer;
use definitions::rsk_provider::RskProvider;
use log::debug;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    store: S,
    rsk_provider: P,
    initial_block_hash: String,
    shutdown_flag: Arc<AtomicBool>,
}

// TODO(iago) Important! Reorgs!
impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: &str,
        shutdown_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            store,
            rsk_provider: provider,
            initial_block_hash: initial_block_hash.to_string(),
            shutdown_flag,
        }
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.load(Ordering::SeqCst)
    }
}

impl<P: RskProvider, S: LogStore> RskIndexer<P, S> for LogIndexer<P, S> {
    fn run(&self) -> Result<()> {
        while self.is_running() {
            debug!("New log received...");
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        Ok(())
    }
}
