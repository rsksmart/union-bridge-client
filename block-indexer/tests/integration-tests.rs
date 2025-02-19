use anyhow::{anyhow, Result};
use common::rsk_provider::{MockRskProvider, MockRskSubscription};
use log::info;
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

/*
# Given the initial best block is B
# And the provider initially has blocks B to N
# And the provider can retrieve blocks N+1 to Z under subscription
# When the indexer is started
# Then the indexer should reach block Z
*/
#[test]
fn test_when_monitor_runs_should_backwards_sync_and_add_blocks_from_subscription() -> Result<()> {
    const BLOCK_CACHE_SIZE: usize = 100;
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 6;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 50;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 5;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    {
        let store: CachedBlockStore<LruCache<RskBlock>> =
            CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
        let mut mock_rsk_provider = MockRskProvider::new();
        let generator = rsk::FakeBlockGenerator::new();

        let generator_clone = generator.clone();
        // in initialize_db_if_required, it will set initial_block_node with the block INIT_BLOCK_HEIGHT
        mock_rsk_provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq(""))
            .returning(move |_hash| Ok(Some(generator_clone.generate_block(INIT_BLOCK_HEIGHT))));

        let generator_clone = generator.clone();
        // every time the indexer queries the best block from the provider, it will get block number MAX_BLOCK_HEIGHT_BACKWARDS_SYNC
        mock_rsk_provider
            .expect_get_best_block()
            .returning(move || Ok(generator_clone.generate_block(MAX_BLOCK_HEIGHT_BACKWARDS_SYNC)));

        let generator_clone = generator.clone();
        // every time the indexer queries a block from the provider, it will get the requested block
        mock_rsk_provider.expect_get_block_by_number().returning({
            move |num| {
                if (INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC).contains(&num) {
                    Ok(Some(generator_clone.generate_block(num)))
                } else {
                    Ok(None)
                }
            }
        });

        let mut counter = MAX_BLOCK_HEIGHT_BACKWARDS_SYNC + 1;
        let shutdown_flag = ShutdownFlag::init();
        let generator_clone = generator.clone();
        let shutdown_flag_for_sub = shutdown_flag.clone();
        // when the indexer subscribes to blocks, it will start receiving blocks with a slight delay between them
        mock_rsk_provider.expect_subscribe_blocks().returning({
            move |_shutdown_flag| {
                let mut mock_sub = MockRskSubscription::<RskBlock>::new();
                let generator_clone = generator_clone.clone();
                let shutdown_flag_clone = shutdown_flag_for_sub.clone();
                mock_sub.expect_next().returning({
                    move || {
                        thread::sleep(Duration::from_millis(DELAY_BETWEEN_BLOCKS_SUBSCRIPTION));
                        let block = generator_clone.generate_block(counter);
                        counter += 1;
                        if counter <= MAX_BLOCK_HEIGHT_SUBSCRIPTION {
                            Ok(block)
                        } else {
                            while !shutdown_flag_clone.is_on() {
                                thread::sleep(Duration::from_millis(20));
                            }
                            Ok(block)
                        }
                    }
                });
                mock_sub.expect_unsubscribe().returning(|| Ok(()));
                Ok(mock_sub)
            }
        });

        let indexer = BlockIndexer::new(store, mock_rsk_provider, "", shutdown_flag.clone());
        {
            let shutdown_flag_clone = shutdown_flag.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(500));
                shutdown_flag_clone.set(true);
            });
        }
        indexer.run()?;
        info!("Indexer run completed successfully.");
    }

    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let best_block = store_after
        .get_best_block()?
        .ok_or_else(|| anyhow!("No best block found after indexer run"))?;

    assert_eq!(
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
        best_block.number(),
        "Best block after subscription should be {}",
        MAX_BLOCK_HEIGHT_SUBSCRIPTION
    );
    Ok(())
}

/*
# Given the initial best block is B
# And the provider initially has blocks 1 to Z
# When the indexer is started
# And the shutdown flag is set after block N
# Then the store should contain blocks B to N
# And the store should contain a checkpoint at block N
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_should_set_checkpoint() -> Result<()> {
    const BLOCK_CACHE_SIZE: usize = 200;
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 100;
    const BLOCK_HEIGHT_WHEN_SHUTDOWN: u64 = 20;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    {
        let store: CachedBlockStore<LruCache<RskBlock>> =
            CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
        let mut mock_rsk_provider = MockRskProvider::new();
        let generator = rsk::FakeBlockGenerator::new();

        let generator_clone = generator.clone();
        // in initialize_db_if_required, it will set initial_block_node with the block INIT_BLOCK_HEIGHT
        mock_rsk_provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq(""))
            .returning(move |_hash| Ok(Some(generator_clone.generate_block(INIT_BLOCK_HEIGHT))));

        let generator_clone = generator.clone();
        // every time the indexer queries the best block from the provider, it will get block number MAX_BLOCK_HEIGHT_BACKWARDS_SYNC
        mock_rsk_provider
            .expect_get_best_block()
            .returning(move || Ok(generator_clone.generate_block(MAX_BLOCK_HEIGHT_BACKWARDS_SYNC)));

        let generator_clone = generator.clone();
        let shutdown_flag = ShutdownFlag::init();
        let shutdown_flag_clone = shutdown_flag.clone();

        // every time the indexer queries a block from the provider, it will get the requested block
        mock_rsk_provider.expect_get_block_by_number().returning({
            move |num| {
                if (INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC).contains(&num) {
                    if num == BLOCK_HEIGHT_WHEN_SHUTDOWN {
                        shutdown_flag_clone.set(true);
                    }
                    Ok(Some(generator_clone.generate_block(num)))
                } else {
                    Ok(None)
                }
            }
        });

        let indexer = BlockIndexer::new(store, mock_rsk_provider, "", shutdown_flag.clone());
        indexer.run()?;
        info!("Indexer run completed successfully.");
    }

    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let checkpoint_block = store_after
        .get_back_sync_checkpoint()?
        .ok_or_else(|| anyhow!("No checkpoint block found after indexer run"))?;

    assert_eq!(
        BLOCK_HEIGHT_WHEN_SHUTDOWN,
        checkpoint_block.number(),
        "Checkpoint block after interruption of backward sync should be {}",
        BLOCK_HEIGHT_WHEN_SHUTDOWN
    );
    Ok(())
}
