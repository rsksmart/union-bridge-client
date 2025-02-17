use anyhow::{anyhow, Result};
use common::rsk_provider::{MockRskProvider, MockRskSubscription};
use log::info;
use std::env;
use std::fs;
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
    const INITIAL_STORE_BEST_BLOCK: &str =
        "0xd86e8112f3c4c4442126f8e9f44f16867da487f29052bf91b810457db34209a4";

    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    env::set_var(INITIAL_BLOCK_HASH, "0x7c9fa136d4413fa6173637e883b6998d32e1d675f88cddff9dcbcf331820f4b8");
    env::set_var(STORE_PATH, store_path);
    env::set_var(BLOCK_CACHE_SIZE, "20");

    {
        let store: CachedBlockStore<LruCache<RskBlock>> = CachedBlockStore::new(store_path)?;
        let mut mock_rsk_provider = MockRskProvider::new();
        let generator = rsk::FakeBlockGenerator::new();

        let generator_clone = generator.clone();
        mock_rsk_provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq(INITIAL_STORE_BEST_BLOCK))
            .returning(move |_hash| Ok(Some(generator_clone.generate_block(2))));

        let generator_clone = generator.clone();
        mock_rsk_provider
            .expect_get_best_block()
            .returning(move || Ok(generator_clone.generate_block(6)));

        let generator_clone = generator.clone();
        mock_rsk_provider.expect_get_block_by_number().returning({
            move |num| {
                if (2..=5).contains(&num) {
                    Ok(Some(generator_clone.generate_block(num)))
                } else {
                    Ok(None)
                }
            }
        });

        let mut counter = 6;
        let generator_clone = generator.clone();
        mock_rsk_provider.expect_subscribe_blocks().returning({
            move |_shutdown_flag| {
                let mut mock_sub = MockRskSubscription::<RskBlock>::new();
                let generator_clone = generator_clone.clone();
                mock_sub.expect_next().returning({
                    move || {
                        thread::sleep(Duration::from_millis(20));
                        let block = generator_clone.generate_block(counter);
                        counter += 1;
                        Ok(block)
                    }
                });
                mock_sub.expect_unsubscribe().returning(|| Ok(()));
                Ok(mock_sub)
            }
        });

        let shutdown_flag = ShutdownFlag::init();
        let indexer = BlockIndexer::new(
            store,
            mock_rsk_provider,
            INITIAL_STORE_BEST_BLOCK,
            shutdown_flag.clone(),
        );
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
