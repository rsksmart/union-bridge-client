use anyhow::{anyhow, Result};
use common::rsk_provider::MockRskProvider;
use common::test_utils::rsk::{
    set_provider_expect_get_best_block, set_provider_expect_get_block_by_hash,
    set_provider_expect_get_block_by_number,
    set_provider_expect_get_block_by_number_with_shutdown_at_block,
    set_provider_expect_subscribe_blocks,
};
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
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = rsk::FakeBlockGenerator::new();
    let shutdown_flag = ShutdownFlag::init();

    set_provider_expect_get_block_by_hash(
        &mut mock_rsk_provider,
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_get_block_by_number(
        &mut mock_rsk_provider,
        &generator,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_subscribe_blocks(
        &mut mock_rsk_provider,
        &generator,
        &shutdown_flag,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC + 1,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let indexer = BlockIndexer::new(store, mock_rsk_provider, "", shutdown_flag.clone());
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        shutdown_flag.clone().set(true);
    });
    indexer.run()?;
    info!("Indexer run completed successfully.");
    drop(indexer); // so it releases the store lock
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
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = rsk::FakeBlockGenerator::new();

    set_provider_expect_get_block_by_hash(
        &mut mock_rsk_provider,
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    let shutdown_flag = ShutdownFlag::init();
    set_provider_expect_get_block_by_number_with_shutdown_at_block(
        &mut mock_rsk_provider,
        &generator,
        &shutdown_flag,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        BLOCK_HEIGHT_WHEN_SHUTDOWN,
    );
    let indexer = BlockIndexer::new(store, mock_rsk_provider, "", shutdown_flag.clone());
    indexer.run()?;
    info!("Indexer run completed successfully.");
    drop(indexer); // so it releases the store lock
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

/*
# Given the initial best block is B
# And the provider initially has blocks 1 to Z
# When the indexer is started
# And the shutdown flag is set after block N
# And the indexer is started again
# Then the indexer should start from block N+1
# And the store should clear the checkpoint
# And the indexer should reach block Z
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_and_indexer_restarts_should_complete_sync_and_add_blocks_from_subscription(
) -> Result<()> {
    const BLOCK_CACHE_SIZE: usize = 200;
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const BLOCK_HEIGHT_CHECKPOINT: u64 = 10;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 100;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 5;

    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let generator = rsk::FakeBlockGenerator::new();

    // Phase 1: Run indexer and simulate shutdown during backward sync
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    let shutdown_flag = ShutdownFlag::init();
    set_provider_expect_get_block_by_hash(
        &mut mock_rsk_provider,
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_get_block_by_number_with_shutdown_at_block(
        &mut mock_rsk_provider,
        &generator,
        &shutdown_flag,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        BLOCK_HEIGHT_CHECKPOINT,
    );
    let indexer = BlockIndexer::new(store, mock_rsk_provider, "", shutdown_flag);
    indexer.run()?;
    info!("Phase 1 (backward sync up to checkpoint) completed successfully.");
    drop(indexer); // so it releases the store lock
                   // End of Phase 1

    // Phase 2: Recovery and subscription
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    let shutdown_flag = ShutdownFlag::init();
    let checkpoint_parent_hash_string = generator
        .clone()
        .generate_block(BLOCK_HEIGHT_CHECKPOINT - 1)
        .hash()
        .to_string();
    set_provider_expect_get_block_by_hash(
        &mut mock_rsk_provider,
        checkpoint_parent_hash_string,
        &generator,
        INIT_BLOCK_HEIGHT,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_get_block_by_number(
        &mut mock_rsk_provider,
        &generator,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_subscribe_blocks(
        &mut mock_rsk_provider,
        &generator,
        &shutdown_flag,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC + 1,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let indexer = BlockIndexer::new(store, mock_rsk_provider, "", shutdown_flag.clone());
    {
        // shutdown after some time
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(600));
            shutdown_flag.clone().set(true);
        });
    }
    indexer.run()?;
    info!("Phase 2 (checkpoint recovery and subscription) completed successfully.");
    drop(indexer); // so it releases the store lock
                   // End of Phase 2

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
    let checkpoint_block = store_after.get_back_sync_checkpoint()?;
    assert_eq!(
        None, checkpoint_block,
        "Checkpoint block should be cleared after indexer run"
    );
    Ok(())
}
