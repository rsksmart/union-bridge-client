use anyhow::{anyhow, Result};
use common::rsk_provider::{MockRskProvider, MockRskSubscription};
use log::info;
use std::env;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

use block_indexer::indexer::BlockIndexer;
use block_indexer::store::{BlockStore, CachedBlockStore};
use common::cache::LruCache;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::test_utils::rsk;
use common::types::RskBlock;

#[test]
fn test_when_monitor_runs_should_backwards_sync_and_add_blocks_from_subscription() -> Result<()> {
    const INITIAL_BLOCK_HASH: &str = "INITIAL_BLOCK_HASH";
    const STORE_PATH: &str = "STORE_PATH";
    const BLOCK_CACHE_SIZE: &str = "BLOCK_CACHE_SIZE";

    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    env::set_var(INITIAL_BLOCK_HASH, "1");
    env::set_var(STORE_PATH, store_path);
    env::set_var(BLOCK_CACHE_SIZE, "20");

    {
        let store: CachedBlockStore<LruCache<RskBlock>> = CachedBlockStore::new(store_path)?;
        let mut mock_rsk_provider = MockRskProvider::new();
        let generator = Arc::new(Mutex::new(rsk::FakeBlockGenerator::new()));

        mock_rsk_provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq("2"))
            .returning({
                let generator = Arc::clone(&generator);
                move |_hash| {
                    let mut gen = generator.lock().unwrap();
                    Ok(Some(gen.generate_block(2)))
                }
            });

        mock_rsk_provider.expect_get_best_block().returning({
            let generator = Arc::clone(&generator);
            move || {
                let mut gen = generator.lock().unwrap();
                Ok(gen.generate_block(6))
            }
        });

        mock_rsk_provider.expect_get_block_by_number().returning({
            let generator = Arc::clone(&generator);
            move |num| {
                let mut gen = generator.lock().unwrap();
                if (2..=5).contains(&num) {
                    Ok(Some(gen.generate_block(num)))
                } else {
                    Ok(None)
                }
            }
        });

        mock_rsk_provider.expect_subscribe_blocks().returning({
            let generator = Arc::clone(&generator);
            move |_shutdown_flag| {
                let counter = Arc::new(Mutex::new(6));
                let counter_clone = Arc::clone(&counter);
                let mut mock_sub = MockRskSubscription::<RskBlock>::new();
                mock_sub.expect_next().returning({
                    let generator = Arc::clone(&generator);
                    let counter_clone = Arc::clone(&counter_clone);
                    move || {
                        thread::sleep(Duration::from_millis(20));
                        let mut count = counter_clone.lock().unwrap();
                        let mut gen = generator.lock().unwrap();
                        let block = gen.generate_block(*count);
                        *count += 1;
                        Ok(block)
                    }
                });
                mock_sub.expect_unsubscribe().returning(|| Ok(()));
                Ok(mock_sub)
            }
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

    let store_after: CachedBlockStore<LruCache<RskBlock>> = CachedBlockStore::new(store_path)?;
    let best_block = store_after
        .get_best_block()?
        .ok_or_else(|| anyhow!("No best block found after indexer run"))?;

    assert!(
        best_block.number() > 2,
        "Expected best block number > 2, got {}",
        best_block.number()
    );
    Ok(())
}
