use crate::store::LogStore;
use anyhow::Result;
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::RskProvider;
use common::shutdown_flag::ShutdownFlag;
use log::debug;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    _store: S,
    _rsk_provider: P,
    _initial_block_hash: String,
    shutdown_flag: ShutdownFlag,
}

// TODO(iago) Important! Reorgs!
impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: &str,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            _store: store,
            _rsk_provider: provider,
            _initial_block_hash: initial_block_hash.to_string(),
            shutdown_flag,
        }
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
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
