use anyhow::Result;
use common::rsk_provider::MockRskProvider;

use block_indexer::indexer::BlockIndexer;
use block_indexer::store::{BlockStore, CachedBlockStore};
use common::cache::LruCache;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::types::{BlockHash, BlockNumber, RskBlock};
use log::info;
use std::fs;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use tempfile::tempdir;
use test_utils::{
    mock_rsk_provider_handler::MockRskProviderHandler, rsk_block_generator::FakeBlockGenerator,
};

const BLOCK_CACHE_SIZE: usize = 100;
use test_utils::rsk_utils::DEFAULT_BLOCK_HASH;

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N
# And the provider retrieves blocks N+1 to Z under subscription
# When the indexer is started
# Then the best block in the storage should be Z
# And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_monitor_runs_should_backwards_sync_and_add_blocks_from_subscription() -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 40;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let generator = FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(None, None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
    drop(mock_rsk_provider_handler);
    cycle_indexer(store, mock_rsk_provider, shutting_down, None);
    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    assert_best_block(&generator, &store_after, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_canonical_chain(
        &generator,
        &store_after,
        INIT_BLOCK_HEIGHT,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
    );
    Ok(())
}

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N
# When the indexer is started
# And the shutdown flag is set after block H
# Then the storage should contain a checkpoint at block H
# And the best block in the storage should be B
# And the storage should reflect the expected canonical chain containing blocks from H to N
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_should_set_checkpoint() -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT: u64 = 10;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let generator = FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        0.into(),
        0,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(
        None,
        Some(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT.into()),
    );
    drop(mock_rsk_provider_handler);
    cycle_indexer(store, mock_rsk_provider, shutting_down, None);
    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    assert_checkpoint(&generator, &store_after, BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT);
    assert_best_block(&generator, &store_after, INIT_BLOCK_HEIGHT);
    assert_canonical_chain(
        &generator,
        &store_after,
        BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT,
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC,
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
# Then the storage should not have a checkpoint
# And the best block in the storage should be Z
# And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_and_indexer_restarts_should_complete_sync(
) -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 40;
    const BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT: u64 = 10;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let generator = FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));

    // Phase 1: Run indexer and simulate shutdown during backward sync
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(
        None,
        Some(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT.into()),
    );
    drop(mock_rsk_provider_handler);
    cycle_indexer(
        store,
        mock_rsk_provider,
        shutting_down,
        Some("Phase 1 (backward sync up to checkpoint) completed successfully."),
    );

    // Phase 2: Recovery and subscription
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let shutting_down = ShutdownFlag::init();
    let checkpoint_parent_hash_string = generator
        .clone()
        .generate_block(BlockNumber::from(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT) - 1)
        .hash()
        .to_string();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let block_hash = BlockHash::try_from(checkpoint_parent_hash_string.as_str())?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(None, None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
    drop(mock_rsk_provider_handler);
    cycle_indexer(
        store,
        mock_rsk_provider,
        shutting_down,
        Some("Phase 2 (checkpoint recovery and subscription) completed successfully."),
    );

    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    assert_eq!(
        None,
        store_after.get_back_sync_checkpoint()?,
        "Checkpoint block should be cleared after indexer run"
    );
    assert_best_block(&generator, &store_after, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_canonical_chain(
        &generator,
        &store_after,
        INIT_BLOCK_HEIGHT,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
    );
    Ok(())
}

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N (B < N)
# And the provider retrieves blocks N+1 to Z under subscription (N < Z)
# When the indexer is started
# And a reorg happens at block K, from block H (B < H < K < N)
# Then the best block in the storage should be Z
# And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_backwards_sync_should_complete_sync(
) -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 40;
    const REORG_BLOCK_HEIGHT: u64 = 10;
    const REORG_HAPPENS_AT_HEIGHT: u64 = 15;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let shutting_down = ShutdownFlag::init();
    let is_reorg = Arc::new(AtomicBool::new(false));
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let generator = FakeBlockGenerator::new(REORG_BLOCK_HEIGHT.into(), is_reorg.clone());
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
    drop(mock_rsk_provider_handler);
    cycle_indexer(store, mock_rsk_provider, shutting_down, None);
    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    assert_best_block(&generator, &store_after, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_canonical_chain(
        &generator,
        &store_after,
        INIT_BLOCK_HEIGHT,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
    );
    Ok(())
}

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N (B < N)
# And the provider retrieves blocks N+1 to Z under subscription (N < Z)
# When the indexer is started
# And a reorg happens at block X, from block P (N < P < X < Z)
# Then the best block in the storage should be Z
# And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_subscription_should_complete_sync() -> Result<()>
{
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 40;
    const REORG_BLOCK_HEIGHT: u64 = 25;
    const REORG_HAPPENS_AT_HEIGHT: u64 = 30;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let shutting_down = ShutdownFlag::init();
    let is_reorg = Arc::new(AtomicBool::new(false));
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let generator = FakeBlockGenerator::new(REORG_BLOCK_HEIGHT.into(), is_reorg.clone());
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler
        .set_provider_expect_subscribe_blocks(Some(REORG_HAPPENS_AT_HEIGHT.into()));
    drop(mock_rsk_provider_handler);
    cycle_indexer(store, mock_rsk_provider, shutting_down, None);
    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    assert_best_block(&generator, &store_after, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_canonical_chain(
        &generator,
        &store_after,
        INIT_BLOCK_HEIGHT,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
    );
    Ok(())
}

/*
# Given the initial best block is B
# And the storage is empty
# And the provider has blocks B to N (B < N)
# And the provider retrieves blocks N+1 to Z under subscription (N < Z)
# When the indexer is started
# And a reorg happens at block X, from block H (B < H < N < X < Z)
# Then the best block in the storage should be Z
# And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_subscription_from_early_block_should_complete_sync(
) -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_BACKWARDS_SYNC: u64 = 20;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 40;
    const REORG_BLOCK_HEIGHT: u64 = 10;
    const REORG_HAPPENS_AT_HEIGHT: u64 = 30;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().join("blocks");
    fs::create_dir_all(&store_path)?;
    let store_path: &str = store_path.to_str().unwrap();
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let shutting_down = ShutdownFlag::init();
    let is_reorg = Arc::new(AtomicBool::new(false));
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));
    let generator = FakeBlockGenerator::new(REORG_BLOCK_HEIGHT.into(), is_reorg.clone());
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &generator,
        None,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler
        .set_provider_expect_subscribe_blocks(Some(REORG_HAPPENS_AT_HEIGHT.into()));
    drop(mock_rsk_provider_handler);
    cycle_indexer(store, mock_rsk_provider, shutting_down, None);
    let store_after: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    assert_best_block(&generator, &store_after, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_canonical_chain(
        &generator,
        &store_after,
        INIT_BLOCK_HEIGHT,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
    );
    Ok(())
}

fn cycle_indexer(
    store: CachedBlockStore<LruCache<RskBlock>>,
    mock_rsk_provider: Arc<Mutex<MockRskProvider>>,
    shutting_down: ShutdownFlag,
    msg: Option<&str>,
) -> () {
    let mock_rsk_provider = Arc::try_unwrap(mock_rsk_provider)
        .unwrap()
        .into_inner()
        .unwrap();
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH).expect("Invalid hex string");
    let indexer = BlockIndexer::new(store, mock_rsk_provider, block_hash, shutting_down.clone());
    let _ = indexer.run();
    info!("{}", msg.unwrap_or("Indexer run completed successfully."));
    drop(indexer);
}

fn assert_best_block(
    generator: &FakeBlockGenerator,
    store_after: &CachedBlockStore<LruCache<RskBlock>>,
    best_block_height: u64,
) -> () {
    let best_block = store_after
        .get_best_block()
        .unwrap_or_else(|err| panic!("Failed to retrieve best block: {}", err))
        .expect("No best block found after indexer run");
    let block_expected = generator.generate_block(best_block_height.into());
    assert_eq!(
        block_expected.hash(),
        best_block.hash(),
        "Hash of best block in storage does not match the hash of the expected block (height {})",
        best_block_height
    );
    assert_eq!(
        block_expected.number(),
        best_block.number(),
        "Height of best block in storage does not match the height of the expected block ({})",
        best_block_height
    );
}

fn assert_checkpoint(
    generator: &FakeBlockGenerator,
    store_after: &CachedBlockStore<LruCache<RskBlock>>,
    checkpoint_block_height: u64,
) -> () {
    let checkpoint_block = store_after
        .get_back_sync_checkpoint()
        .unwrap_or_else(|err| panic!("Failed to retrieve checkpoint block: {}", err))
        .expect("No checkpoint block found after indexer run");
    let block_expected = generator.generate_block(checkpoint_block_height.into());
    assert_eq!(
        block_expected.hash(),
        checkpoint_block.hash(),
        "Hash of checkpoint block in storage does not match the hash of the expected block (height {})", checkpoint_block_height
    );
    assert_eq!(
        block_expected.number(),
        checkpoint_block.number(),
        "Height of checkpoint block in storage does not match the height of the expected block ({})", checkpoint_block_height
    );
}

fn assert_canonical_chain(
    generator: &FakeBlockGenerator,
    store_after: &CachedBlockStore<LruCache<RskBlock>>,
    begin_height: u64,
    end_height: u64,
) -> () {
    for height in begin_height..=end_height {
        let block_expected = generator.clone().generate_block(height.into());
        let block_actual = store_after
            .get_canonical_block(height.into())
            .unwrap_or_else(|err| panic!("Failed to retrieve canonical block: {}", err))
            .expect(&format!(
                "No canonical block at height {} found after indexer run",
                height
            ));
        assert_eq!(
            block_expected.hash(),
            block_actual.hash(),
            "Hash of canonical block in storage at height {} does not match the hash of the expected block",
            height
        );
    }
}
