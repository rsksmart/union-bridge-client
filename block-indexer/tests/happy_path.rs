
use anyhow::{Result, anyhow};
use common::rsk_provider::MockRskProvider;
use std::env;
use std::fs;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;
use log::info;

use common::types::RskBlock;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::cache::LruCache;
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::{CachedBlockStore, BlockStore};

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

#[test]
fn test_when_monitor_runs() -> Result<()> {
    env::set_var("INITIAL_BLOCK_HASH", "1");

    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path_str: &str = store_path.to_str().unwrap();
    env::set_var("STORE_PATH", store_path_str);
    env::set_var("BLOCK_CACHE_SIZE", "20");

    {
        let store: CachedBlockStore<LruCache<RskBlock>> = CachedBlockStore::new(store_path_str)?;
        let mut mock_rsk_provider = MockRskProvider::new();

        mock_rsk_provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq("2"))
            .returning(|_hash| Ok(Some(create_dummy_block(2))));

        mock_rsk_provider
        .expect_get_best_block()
        .returning(|| Ok(create_dummy_block(6)));

        mock_rsk_provider
            .expect_get_block_by_number()
            .returning(|num| {
                if (2..=5).contains(&num) {
                    Ok(Some(create_dummy_block(num)))
                } else {
                    Ok(None)
                }
            });

        use std::sync::{Arc, Mutex};
        mock_rsk_provider
            .expect_subscribe_blocks()
            .returning(|_shutdown_flag| {
                let counter = Arc::new(Mutex::new(6));
                let counter_clone = counter.clone();
                let mut mock_sub = common::rsk_provider::MockRskSubscription::<RskBlock>::new();
                mock_sub.expect_next().returning(move || {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    let mut count = counter_clone.lock().unwrap();
                    let block = create_dummy_block(*count);
                    *count += 1;
                    Ok(block)
                });
                mock_sub.expect_unsubscribe().returning(|| Ok(()));
                Ok(mock_sub)
            });

        let shutdown_flag = ShutdownFlag::init();
        let indexer = BlockIndexer::new(store, mock_rsk_provider, "2", shutdown_flag.clone());
        {
            let shutdown_flag_clone = shutdown_flag.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(300));
                shutdown_flag_clone.set(true);
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