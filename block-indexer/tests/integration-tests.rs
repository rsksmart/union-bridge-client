use anyhow::{anyhow, Result};
use common::rsk_provider::MockRskProvider;
use common::test_utils::rsk::{
    set_provider_expect_get_best_block, set_provider_expect_get_block_by_hash,
    set_provider_expect_get_block_by_number, set_provider_expect_get_block_by_number_with_reorg,
    set_provider_expect_get_block_by_number_with_shutdown_at_block,
    set_provider_expect_subscribe_blocks,
};
use log::info;
use std::fs;
use std::sync::{atomic::AtomicBool, Arc};
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

const BLOCK_CACHE_SIZE: usize = 100;

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N
# And the provider retrieves blocks N+1 to Z under subscription
# When the indexer is started
# Then the storage should reach block Z
*/
#[test]
fn test_when_monitor_runs_should_backwards_sync_and_add_blocks_from_subscription() -> Result<()> {
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
        Arc::new(AtomicBool::new(false)),
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
        0,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        0,
    );
    set_provider_expect_get_block_by_number(
        &mut mock_rsk_provider,
        &generator,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_subscribe_blocks(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        &generator,
        &shutdown_flag,
        0,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
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
# And the storage is empty
# And the provider has blocks B to N
# When the indexer is started
# And the shutdown flag is set after block H
# Then the storage should contain blocks B to H
# And the storage should contain a checkpoint at block H
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_should_set_checkpoint() -> Result<()> {
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 100;
    const BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT: u64 = 20;
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
        Arc::new(AtomicBool::new(false)),
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
        0,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        0,
    );
    let shutdown_flag = ShutdownFlag::init();
    set_provider_expect_get_block_by_number_with_shutdown_at_block(
        &mut mock_rsk_provider,
        &generator,
        &shutdown_flag,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT,
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
        BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT,
        checkpoint_block.number(),
        "Checkpoint block after interruption of backward sync should be {}",
        BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT
    );
    Ok(())
}

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N
# And the provider retrieves blocks N+1 to Z under subscription
# When the indexer is started
# And the shutdown flag is set at block H
# And the indexer is started again
# Then the store should clear the checkpoint
# And the storage should reach block Z
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_and_indexer_restarts_should_complete_sync(
) -> Result<()> {
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT: u64 = 10;
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
        Arc::new(AtomicBool::new(false)),
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
        0,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        0,
    );
    set_provider_expect_get_block_by_number_with_shutdown_at_block(
        &mut mock_rsk_provider,
        &generator,
        &shutdown_flag,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT,
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
        .generate_block(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT - 1, 0, false)
        .hash()
        .to_string();
    set_provider_expect_get_block_by_hash(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        checkpoint_parent_hash_string,
        &generator,
        INIT_BLOCK_HEIGHT,
        0,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        0,
    );
    set_provider_expect_get_block_by_number(
        &mut mock_rsk_provider,
        &generator,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
    );
    set_provider_expect_subscribe_blocks(
        &mut mock_rsk_provider,
        Arc::new(AtomicBool::new(false)),
        &generator,
        &shutdown_flag,
        0,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
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

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N
# And the provider retrieves blocks N+1 to Z under subscription
# When the indexer is started
# And a reorg happens at block H
# Then the storage should reach block N
# And the storage should contain reorged blocks from block H
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_backwards_sync_should_complete_sync(
) -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const REORG_BLOCK_HEIGHT: u64 = 10;
    const BLOCK_REORG_HAPPENS_AT: u64 = 20;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 30;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 40;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 5;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let shutdown_flag = ShutdownFlag::init();
    let is_reorg = Arc::new(AtomicBool::new(false));

    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = rsk::FakeBlockGenerator::new();
    set_provider_expect_get_block_by_hash(
        &mut mock_rsk_provider,
        Arc::clone(&is_reorg),
        "".to_string(),
        &generator,
        INIT_BLOCK_HEIGHT,
        REORG_BLOCK_HEIGHT,
    );
    set_provider_expect_get_best_block(
        &mut mock_rsk_provider,
        Arc::clone(&is_reorg),
        &generator,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        REORG_BLOCK_HEIGHT,
    );
    set_provider_expect_get_block_by_number_with_reorg(
        &mut mock_rsk_provider,
        Arc::clone(&is_reorg),
        &generator,
        INIT_BLOCK_HEIGHT..=MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
        BLOCK_REORG_HAPPENS_AT,
        REORG_BLOCK_HEIGHT,
    );
    set_provider_expect_subscribe_blocks(
        &mut mock_rsk_provider,
        Arc::clone(&is_reorg),
        &generator,
        &shutdown_flag,
        REORG_BLOCK_HEIGHT,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
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
    for height in INIT_BLOCK_HEIGHT..MAX_BLOCK_HEIGHT_SUBSCRIPTION {
        let block_expected = generator
            .clone()
            .generate_block(height, REORG_BLOCK_HEIGHT, true);
        let block_actual = store_after.get_canonical_block(height)?.ok_or_else(|| {
            anyhow!(
                "No canonical block at height {} found after indexer run",
                height
            )
        })?;
        assert_eq!(
            block_expected.hash(),
            block_actual.hash(),
            "Hash of canonical block in storage at height {} does not match the hash of the expected block",
            height
        );
    }
    Ok(())
}
