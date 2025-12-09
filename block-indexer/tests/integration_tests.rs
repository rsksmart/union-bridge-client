#![cfg(not(feature = "fresh_node"))]

use std::fs;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::{BlockStore, CachedBlockStore};
use common::cache::LruCache;
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::MockRskProvider;
use common::shutdown_flag::ShutdownFlag;
use common::test_utils::mock_rsk_provider_handler::MockRskProviderHandler;
use common::test_utils::rsk_block_generator::FakeBlockGenerator;
use common::types::{BlockHash, BlockNumber, RskBlock};
use log::info;
use tempfile::tempdir;
const BLOCK_CACHE_SIZE: usize = 100;
use common::test_utils::rsk_utils::{DEFAULT_BLOCK_HASH, UncleBlockInfo};

/*
Scenario: happy path
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And the provider retrieves blocks N to Z under subscription
# (M+1 = N)
When the indexer is started
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(None, None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
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
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
When the indexer is started
And the shutdown flag is set at block H
# (H < M)
Then the storage should contain a checkpoint at block H
And the best block in the storage should be B
And the storage should reflect the expected canonical chain containing blocks from H to M
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        0.into(),
        0,
        None,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(
        None,
        Some(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT.into()),
    );
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
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And the provider retrieves blocks N to Z under subscription
# (M+1 = N)
When the indexer is started
And the shutdown flag is set at block H
# (H < M)
And the indexer is started again
Then the storage should not have a checkpoint
And the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_shutdown_happens_during_backwards_sync_and_indexer_restarts_should_complete_sync()
-> Result<()> {
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
    let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);

    // Phase 1: Run indexer and simulate shutdown during backward sync
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(
        None,
        Some(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT.into()),
    );
    cycle_indexer(
        store,
        mock_rsk_provider,
        shutting_down,
        Some("Phase 1 (backward sync up to checkpoint) completed successfully."),
    );

    // Phase 2: Recovery and subscription
    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(store_path, BLOCK_CACHE_SIZE)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    let shutting_down = ShutdownFlag::init();
    let checkpoint_parent_hash_string = generator
        .clone()
        .generate_block(BlockNumber::from(BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT) - 1, None)
        .expect("Failed to generate block")
        .hash()
        .to_string();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    let hash = BlockHash::try_from(checkpoint_parent_hash_string.as_str())?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(hash, BLOCK_HEIGHT_SHUTDOWN_HAPPENS_AT.into());
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(None, None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
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
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And the provider retrieves blocks N to Z under subscription
# (M+1 = N)
When the indexer is started
And a reorg happens at block K, from block H
# (B < H < K < M)
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_backwards_sync_should_complete_sync()
-> Result<()> {
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator =
        FakeBlockGenerator::new(Some(REORG_BLOCK_HEIGHT.into()), is_reorg.clone(), None);
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
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
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And the provider retrieves blocks N to Z under subscription
# (M+1 = N)
When the indexer is started
And a reorg happens at block X, from block P
# (N < P < X < Z)
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator =
        FakeBlockGenerator::new(Some(REORG_BLOCK_HEIGHT.into()), is_reorg.clone(), None);
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler
        .set_provider_expect_subscribe_blocks(Some(REORG_HAPPENS_AT_HEIGHT.into()));
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
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And the provider retrieves blocks N to Z under subscription
# (M+1 = N)
When the indexer is started
And a reorg happens at block X, from block H
# (B < H < N < X < Z)
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_subscription_from_early_block_should_complete_sync()
-> Result<()> {
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator =
        FakeBlockGenerator::new(Some(REORG_BLOCK_HEIGHT.into()), is_reorg.clone(), None);
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler
        .set_provider_expect_subscribe_blocks(Some(REORG_HAPPENS_AT_HEIGHT.into()));
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

/* Happy path with uncles
Given the initial best block is B
And the storage is empty
And the provider retrieves blocks B to M under backward sync
And certain blocks do have uncle blocks
| blockID | uncleBlockIDarr |
| D       | D.A             |
| G       | G.A, G.B        |
And the provider retrieves blocks N to Z under subscription
# (M+1 = N)
And certain blocks do have uncle blocks
| blockID | uncleBlockIDarr |
| P       | P.A, P.B        |
| S       | S.A             |
When the indexer is started
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
And the storage should reflect that appropriate blocks are linked to its uncle blocks
| blockID | uncleBlockIDarr |
| D       | D.A             |
| G       | G.A, G.B        |
| P       | P.A, P.B        |
| S       | S.A             |
And the storage should contain these uncle blocks
| uncleBlockIDarr |
| D.A             |
| G.A, G.B        |
| P.A, P.B        |
| S.A             |
*/
#[test]
fn test_when_monitor_runs_should_backwards_sync_and_add_blocks_from_subscription_with_uncles()
-> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    let uncle_block_info_vec: Vec<UncleBlockInfo> = vec![
        UncleBlockInfo::new(5, false, "uD.A", 0),
        UncleBlockInfo::new(8, false, "uG.A", 0),
        UncleBlockInfo::new(8, false, "uG.B", 1),
        UncleBlockInfo::new(22, false, "uP.A", 0),
        UncleBlockInfo::new(22, false, "uP.B", 1),
        UncleBlockInfo::new(28, false, "uS.A", 0),
    ];
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = FakeBlockGenerator::new(
        None,
        Arc::new(AtomicBool::new(false)),
        Some(uncle_block_info_vec.clone()),
    );
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        Some(uncle_block_info_vec.clone()),
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_uncle_by_hash_and_index();
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler.set_provider_expect_get_block_by_number(None, None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
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
    assert_uncle_block_links(&store_after, INIT_BLOCK_HEIGHT, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_uncle_blocks_in_storage(&generator, &store_after, uncle_block_info_vec.clone());
    Ok(())
}

/* Reorg during backward sync with uncles
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And certain blocks do have uncle blocks in the original chain
| blockID | uncleBlockIDarr |
| D       | D.A             |
| J       | J.A             |
| L       | L.A             |
| P       | P.A             |
| S       | S.A             |
And certain blocks do have uncle blocks in the reorged chain
| blockID | uncleBlockIDarr |
| F       | F.A             |
| J2      | J2.A            |
| L2      | L2.A            |
| P2      | P2.A            |
| T       | T.A             |
And the provider retrieves blocks N to Z under subscription
# (N = M+1)
When the indexer is started
And a reorg happens at block K, from block H (B < H < K < M)
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
And the storage should reflect that appropriate blocks are linked to its uncle blocks
| blockID | uncleBlockIDarr |
| D       | D.A             |
| J2      | J2.A            |
| L2      | L2.A            |
| P2      | P2.A            |
| T       | T.A             |
And the storage should contain these uncle blocks
| uncleBlockIDarr |
| D.A             |
| J2.A            |
| L2.A            |
| P2.A            |
| T.A             |
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_backwards_sync_should_complete_sync_with_uncles()
-> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    let uncle_block_info_vec: Vec<UncleBlockInfo> = vec![
        UncleBlockInfo::new(5, false, "uD.A", 0),
        UncleBlockInfo::new(12, false, "uJ.A", 0),
        UncleBlockInfo::new(17, false, "uL.A", 0),
        UncleBlockInfo::new(22, false, "uP.A", 0),
        UncleBlockInfo::new(28, false, "uS.A", 0),
        UncleBlockInfo::new(8, true, "uF.A", 0),
        UncleBlockInfo::new(12, true, "uJ2.A", 0),
        UncleBlockInfo::new(13, true, "uJJ.A", 0),
        UncleBlockInfo::new(17, true, "uL2.A", 0),
        UncleBlockInfo::new(19, true, "uLL.A", 0),
        UncleBlockInfo::new(22, true, "uP2.A", 0),
        UncleBlockInfo::new(23, true, "uPP.A", 0),
        UncleBlockInfo::new(33, true, "uT.A", 0),
    ];
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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = FakeBlockGenerator::new(
        Some(REORG_BLOCK_HEIGHT.into()),
        is_reorg.clone(),
        Some(uncle_block_info_vec.clone()),
    );
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        Some(uncle_block_info_vec.clone()),
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_uncle_by_hash_and_index();
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler.set_provider_expect_subscribe_blocks(None);
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
    assert_uncle_block_links(&store_after, INIT_BLOCK_HEIGHT, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_uncle_blocks_in_storage(&generator, &store_after, uncle_block_info_vec.clone());
    Ok(())
}

/* Reorg during subscription with uncles
Given the initial best block is B
And the provider retrieves blocks B to M under backward sync
And the provider retrieves blocks N to Z under subscription
# (N = M+1)
And certain blocks do have uncle blocks in the original chain
| blockID | uncleBlockIDarr |
| P       | P.A             |
| R       | R.A             |
| Y       | Y.A             |
And certain blocks do have uncle blocks in the reorged chain
| blockID | uncleBlockIDarr |
| Q       | Q.A             |
| R2      | R2.A            |
| T       | T.A             |
| Z       | Z.A             |
When the indexer is started
And a reorg happens at block X, from block R (N < R < X < Z)
Then the best block in the storage should be Z
And the storage should reflect the expected canonical chain containing blocks from B to Z
And the storage should reflect that appropriate blocks are linked to its uncle blocks
| blockID | uncleBlockIDarr |
| P       | P.A             |
| R2      | R2.A            |
| T       | T.A             |
| Z       | Z.A             |
And the storage should contain these uncle blocks
| uncleBlockIDarr |
| P.A             |
| R2.A            |
| T.A             |
| Z.A             |
*/
#[test]
fn test_when_monitor_runs_and_reorg_happens_during_subscription_should_complete_sync_with_uncles()
-> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    let uncle_block_info_vec: Vec<UncleBlockInfo> = vec![
        UncleBlockInfo::new(23, false, "uP.A", 0),
        UncleBlockInfo::new(28, false, "uR.A", 0),
        UncleBlockInfo::new(32, false, "uY.A", 0),
        UncleBlockInfo::new(24, true, "uQ.A", 0),
        UncleBlockInfo::new(28, true, "uR2.A", 0),
        UncleBlockInfo::new(29, true, "uT.A", 0),
        UncleBlockInfo::new(38, true, "uZ.A", 0),
    ];

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
    let mut mock_rsk_provider = MockRskProvider::new();
    let generator = FakeBlockGenerator::new(
        Some(REORG_BLOCK_HEIGHT.into()),
        is_reorg.clone(),
        Some(uncle_block_info_vec.clone()),
    );
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &generator,
        is_reorg.clone(),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_BACKWARDS_SYNC.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        Some(uncle_block_info_vec.clone()),
    );
    let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH)?;
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(block_hash, INIT_BLOCK_HEIGHT.into());
    mock_rsk_provider_handler.set_provider_expect_get_uncle_by_hash_and_index();
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_number(Some(REORG_HAPPENS_AT_HEIGHT.into()), None);
    mock_rsk_provider_handler
        .set_provider_expect_subscribe_blocks(Some(REORG_HAPPENS_AT_HEIGHT.into()));
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
    assert_uncle_block_links(&store_after, INIT_BLOCK_HEIGHT, MAX_BLOCK_HEIGHT_SUBSCRIPTION);
    assert_uncle_blocks_in_storage(&generator, &store_after, uncle_block_info_vec.clone());
    Ok(())
}

fn cycle_indexer(
    store: CachedBlockStore<LruCache<RskBlock>>,
    mock_rsk_provider: MockRskProvider,
    shutting_down: ShutdownFlag,
    msg: Option<&str>,
) -> () {
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
    let block_expected = generator.generate_block(best_block_height.into(), None).unwrap();
    assert_eq!(
        block_expected, best_block,
        "Best block in storage does not match the expected best block (height {})",
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
    let block_expected = generator.generate_block(checkpoint_block_height.into(), None).unwrap();
    assert_eq!(
        block_expected, checkpoint_block,
        "Checkpoint block in storage does not match the expected checkpoint block (height {})",
        checkpoint_block_height
    );
}

fn assert_canonical_chain(
    generator: &FakeBlockGenerator,
    store_after: &CachedBlockStore<LruCache<RskBlock>>,
    begin_height: u64,
    end_height: u64,
) -> () {
    for height in begin_height..=end_height {
        let block_expected = generator.clone().generate_block(height.into(), None).unwrap();
        let block_actual = store_after
            .get_canonical_block(height.into())
            .unwrap_or_else(|err| panic!("Failed to retrieve canonical block: {}", err))
            .expect(&format!("No canonical block at height {} found after indexer run", height));
        assert_eq!(
            block_expected, block_actual,
            "Canonical block in storage at height {} does not match the expected block",
            height
        );
    }
}

fn assert_uncle_block_links(
    store_after: &CachedBlockStore<LruCache<RskBlock>>,
    begin_height: u64,
    end_height: u64,
) -> () {
    for height in begin_height..=end_height {
        let block_actual = store_after
            .get_canonical_block(height.into())
            .unwrap_or_else(|err| panic!("Failed to retrieve canonical block: {}", err))
            .expect(&format!("No canonical block at height {} found after indexer run", height));
        for uncle_hash in block_actual.uncles() {
            let uncle_block_actual = store_after
                .get_block_by_hash(uncle_hash)
                .unwrap_or_else(|err| panic!("Failed to retrieve uncle block: {}", err));
            assert!(
                uncle_block_actual.is_some(),
                "No uncle block with hash {} for block at height {} found after indexer run",
                uncle_hash,
                height
            );
        }
    }
}

fn assert_uncle_blocks_in_storage(
    generator: &FakeBlockGenerator,
    store_after: &CachedBlockStore<LruCache<RskBlock>>,
    uncle_block_info_vec: Vec<UncleBlockInfo>,
) -> () {
    for uncle_info in uncle_block_info_vec.iter() {
        let height = uncle_info.height;
        let block_expected = generator.clone().generate_block(height, Some(uncle_info));
        if let Some(block_expected) = block_expected {
            let block_expected_hash = block_expected.hash();
            let block_actual = store_after
                .get_block_by_hash(block_expected.hash())
                .unwrap_or_else(|err| panic!("Failed to retrieve uncle block: {}", err))
                .expect(&format!(
                    "No uncle block with hash {} for block at height {} found after indexer run",
                    block_expected_hash, height
                ));
            assert_eq!(
                block_expected, block_actual,
                "Uncle block in storage with hash {} does not match the expected uncle block",
                block_expected_hash
            );
        }
    }
}
