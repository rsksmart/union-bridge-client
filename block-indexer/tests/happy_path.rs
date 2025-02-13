#![cfg(test)]

use anyhow::{Result, anyhow};
use std::env;
use std::fs;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;
use log::info;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use common::types::{RskBlock, RskLog};
use common::rsk_provider::{RskProvider, RskSubscription, RskProviderError};
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::cache::LruCache;
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::CachedBlockStore;
use block_indexer::store::BlockStore;

fn create_dummy_block(num: u64) -> RskBlock {
    RskBlock::new(
        num,
        num.to_string(),
        if num > 0 { (num - 1).to_string() } else { "".to_string() },
        Default::default(),
        0,
        "".to_string(),
        Default::default(),
    )
}

struct FiniteDummySubscription {
    current: u64,
}

impl FiniteDummySubscription {
    fn new() -> Self {
        Self { current: 0 }
    }
}

impl RskSubscription<RskBlock> for FiniteDummySubscription {
    fn next(&mut self) -> Result<RskBlock, RskProviderError> {
        let block = create_dummy_block(self.current);
        self.current += 1;
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(block)
    }

    fn unsubscribe(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

struct DummyLogSubscription;

impl RskSubscription<RskLog> for DummyLogSubscription {
    fn next(&mut self) -> Result<RskLog, RskProviderError> {
        Err(RskProviderError::Closed)
    }

    fn unsubscribe(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

struct ManualRskProvider;

impl RskProvider for ManualRskProvider {
    fn subscribe_blocks(&self, _shutdown_flag: ShutdownFlag) -> Result<impl RskSubscription<RskBlock>> {
        Ok(FiniteDummySubscription::new())
    }

    fn subscribe_logs(&self, _shutdown_flag: ShutdownFlag) -> Result<impl RskSubscription<RskLog>> {
        Ok(DummyLogSubscription)
    }

    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>> {
        if hash == "2" {
            Ok(Some(create_dummy_block(2)))
        } else {
            Ok(None)
        }
    }

    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>> {
        if (2..=6).contains(&num) {
            Ok(Some(create_dummy_block(num)))
        } else {
            Ok(None)
        }
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        Ok(create_dummy_block(6))
    }

    fn disconnect(&self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn test_when_monitor_runs() -> Result<()> {
    env::set_var("INITIAL_BLOCK_HASH", "1");

    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path_str = store_path.to_str().unwrap();
    env::set_var("STORE_PATH", store_path_str);
    env::set_var("BLOCK_CACHE_SIZE", "20");

    { // put this code in a block so it closes and unlocks the store before we open it again and try to read it
        let store: CachedBlockStore<LruCache<RskBlock>> = CachedBlockStore::new(store_path_str)?;
        let manual_provider = ManualRskProvider;
        let shutdown_flag = ShutdownFlag::init();
        let indexer = BlockIndexer::new(store, manual_provider, "2", shutdown_flag.clone());
        {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(500));
                kill(Pid::this(), Signal::SIGINT).expect("Failed to send SIGINT");
            });
        }
        indexer.run()?;
        info!("Indexer run completed successfully.");
    }

    let store_after: CachedBlockStore<LruCache<RskBlock>> = CachedBlockStore::new(store_path_str)?;
    let best_block = store_after.get_best_block()?
        .ok_or_else(|| anyhow!("No best block found after indexer run"))?;

    assert!(best_block.number() > 2, "Expected best block number > 2, got {}", best_block.number());
    Ok(())
}