use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use common_core::types::{Address, BlockHash, BlockNumber, ContractInfo, LogInfo, RskLog, TxHash};
use common_dev::mock_rsk_provider_handler::MockRskProviderHandler;
use common_dev::rsk_block_generator::FakeBlockGenerator;
use common_dev::rsk_log_generator::FakeLogGenerator;
use common_dev::rsk_utils::{
    DEFAULT_BLOCK_HASH, generate_fake_address, generate_fake_addresses,
    generate_fake_managed_contracts,
};
use common_rsk::rsk_indexer::RskIndexer;
use common_rsk::rsk_provider::{MockRskProvider, RskSubscriptionFilter};
use common_runtime::config::{IndexerConfig, IndexerStartFrom};
use common_runtime::shutdown_flag::ShutdownFlag;
use log_indexer::indexer::LogIndexer;
use log_indexer::store::{LogStore, RawLogStore};
use primitive_types::H256;
use rand::RngExt;
use tempfile::tempdir;
use tracing::info;

const LOG_INDEX_RANGE: Range<u64> = 0..20;
const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;

/*
# Given the storage is empty
# And the provider retrieves valid logs under subscription
# When the log indexer runs
# Then the storage should contain all logs from the subscription
*/
#[test]
fn test_when_log_indexer_runs_should_store_logs_from_subscription() -> Result<()> {
    const LOG_INFO_TUPLE_SIZE: u64 = 10;
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const FILTER_BLOCK_FROM_DEPTH: u64 = 10;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const LOG_BLOCK_HEIGHT_RANGE: Range<u64> =
        MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH..MAX_BLOCK_HEIGHT_SUBSCRIPTION;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let log_generator = FakeLogGenerator::new();
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider = MockRskProvider::new();
    mock_rsk_provider.expect_get_logs().returning(|_, _, _| Ok(vec![]));
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    mock_rsk_provider_handler.set_provider_expect_get_block_by_hash(
        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
        INIT_BLOCK_HEIGHT.into(),
    );
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    let addresses: Vec<Address> = generate_fake_addresses(LOG_INFO_TUPLE_SIZE);
    let log_info_tuples: Vec<LogInfo> =
        log_info_tuple_generator(&LOG_BLOCK_HEIGHT_RANGE, LOG_INFO_TUPLE_SIZE, &addresses);
    let filter = RskSubscriptionFilter::new(
        addresses.clone(),
        vec![],
        Some(MAX_BLOCK_HEIGHT_SUBSCRIPTION.into()),
    );
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(
        filter,
        EVENT_SIGNATURE.to_string(),
        log_info_tuples.clone(),
    );
    let managed_contracts = generate_fake_managed_contracts(addresses);
    cycle_indexer(store, mock_rsk_provider, &managed_contracts, &shutting_down, 0, None);
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    assert_logs(&log_generator, &store_after, EVENT_SIGNATURE, log_info_tuples);
    Ok(())
}

/*
# Given the storage is empty
# And the initial block height is B.D
# And the provider retrieves logs L.A to L.Z under subscription
# And one of the logs (L.D) is in a block with height B.A (B.A < B.D)
# When the log indexer runs
# Then the storage should contain logs from L.A to L.Z except L.D
*/
#[test]
fn test_when_log_before_initial_height_should_not_store_log() -> Result<()> {
    const LOG_INFO_TUPLE_SIZE: u64 = 10;
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const FILTER_BLOCK_FROM_DEPTH: u64 = 10;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const LOG_BLOCK_HEIGHT_RANGE: Range<u64> =
        MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH..MAX_BLOCK_HEIGHT_SUBSCRIPTION;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let log_generator = FakeLogGenerator::new();
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider = MockRskProvider::new();
    mock_rsk_provider.expect_get_logs().returning(|_, _, _| Ok(vec![]));
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    mock_rsk_provider_handler.set_provider_expect_get_block_by_hash(
        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
        INIT_BLOCK_HEIGHT.into(),
    );
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    let addresses: Vec<Address> = generate_fake_addresses(LOG_INFO_TUPLE_SIZE);
    let log_info_tuples: Vec<LogInfo> =
        log_info_tuple_generator(&LOG_BLOCK_HEIGHT_RANGE, LOG_INFO_TUPLE_SIZE, &addresses);
    let filter = RskSubscriptionFilter::new(
        addresses.clone(),
        vec![],
        Some(MAX_BLOCK_HEIGHT_SUBSCRIPTION.into()),
    );
    let bad_log_info = LogInfo::new(
        generate_fake_address(LOG_INFO_TUPLE_SIZE + 1),
        BlockHash::from(H256::random()),
        (INIT_BLOCK_HEIGHT - 1).into(),
        TxHash::from(H256::random()),
        1,
        false,
    );
    let mut log_info_tuples_with_bad_log = log_info_tuples.clone();
    log_info_tuples_with_bad_log.push(bad_log_info.clone());
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(
        filter,
        EVENT_SIGNATURE.to_string(),
        log_info_tuples_with_bad_log.clone(),
    );
    let managed_contracts = generate_fake_managed_contracts(addresses);
    cycle_indexer(store, mock_rsk_provider, &managed_contracts, &shutting_down, 0, None);
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    assert_logs(&log_generator, &store_after, EVENT_SIGNATURE, log_info_tuples);
    assert_log_not_in_store(&log_generator, &store_after, EVENT_SIGNATURE, bad_log_info);
    Ok(())
}

/*
# Given the storage is empty
# And the provider retrieves, under subscription, a log for tx T at block B1 (orphaned chain)
# And later the same tx T is re-mined into a different block B2 (canonical chain)
#   (re-emitted with the same address/tx_hash/log_index but a new block hash and number)
# When the log indexer runs
# Then the storage entry for (address, tx_hash, log_index) should hold the reorged (canonical) log
# And the orphaned log should not survive
*/
#[test]
fn test_when_log_reemitted_in_reorged_block_should_keep_single_canonical_log() -> Result<()> {
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const ORPHANED_BLOCK_HEIGHT: u64 = 28;
    const REORGED_BLOCK_HEIGHT: u64 = 30;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let log_generator = FakeLogGenerator::new();
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider = MockRskProvider::new();
    mock_rsk_provider.expect_get_logs().returning(|_, _, _| Ok(vec![]));
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    mock_rsk_provider_handler.set_provider_expect_get_block_by_hash(
        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
        INIT_BLOCK_HEIGHT.into(),
    );
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    let address = generate_fake_address(1);
    let tx_hash = TxHash::from(H256::random());
    // Orphaned and reorged logs share (address, tx_hash, log_index): the same transaction is
    // re-mined into a new canonical block, so only the block hash and number differ.
    let orphaned_log_info = LogInfo::new(
        address,
        BlockHash::from(H256::random()),
        ORPHANED_BLOCK_HEIGHT.into(),
        tx_hash,
        0,
        false,
    );
    let reorged_log_info = LogInfo::new(
        address,
        BlockHash::from(H256::random()),
        REORGED_BLOCK_HEIGHT.into(),
        tx_hash,
        0,
        false,
    );
    let addresses = vec![address];
    let filter = RskSubscriptionFilter::new(
        addresses.clone(),
        vec![],
        Some(MAX_BLOCK_HEIGHT_SUBSCRIPTION.into()),
    );
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(
        filter,
        EVENT_SIGNATURE.to_string(),
        vec![orphaned_log_info.clone(), reorged_log_info.clone()],
    );
    let managed_contracts = generate_fake_managed_contracts(addresses);
    cycle_indexer(store, mock_rsk_provider, &managed_contracts, &shutting_down, 0, None);
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    let log_key = format!("logs/{address}/{tx_hash}/0");
    let stored: RskLog = store_after.get(&log_key)?.expect("Reorged log not found in storage!");
    let orphaned_log = log_generator.generate_log_with_info(EVENT_SIGNATURE, orphaned_log_info);
    let reorged_log = log_generator.generate_log_with_info(EVENT_SIGNATURE, reorged_log_info);
    assert_eq!(stored, reorged_log, "Storage should hold the reorged (canonical) log");
    assert_ne!(stored, orphaned_log, "Orphaned log must not survive the reorg");
    Ok(())
}

/*
# Given the storage already holds a log for tx T at block B (saved by a previous run)
# And a sync checkpoint exists at a later block C, with B within the finality window below C
# And the chain reorged so the provider now returns, for that window, a log with the same
#   address/tx_hash/log_index but a new block hash
# When the log indexer runs and recovery backs off by the finality depth
# Then the stored entry for (address, tx_hash, log_index) should be overwritten with the reorged log
*/
#[test]
#[allow(clippy::too_many_lines)]
fn test_when_reorg_below_checkpoint_should_overwrite_stale_log_on_recovery() -> Result<()> {
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT: u64 = 35;
    const FINALITY_DEPTH: usize = 5;
    const REORG_BLOCK_HEIGHT: u64 = 28;
    const CHECKPOINT_BLOCK_HEIGHT: u64 = 30;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let log_generator = FakeLogGenerator::new();
    let shutting_down = ShutdownFlag::init();
    let reorg_address = generate_fake_address(1);
    let tip_address = generate_fake_address(2);
    let reorg_tx_hash = TxHash::from(H256::random());
    // The stale (pre-reorg) log and the canonical (post-reorg) log share
    // (address, tx_hash, log_index); only the block hash differs.
    let stale_log_info = LogInfo::new(
        reorg_address,
        BlockHash::from(H256::random()),
        REORG_BLOCK_HEIGHT.into(),
        reorg_tx_hash,
        0,
        false,
    );
    let canonical_log_info = LogInfo::new(
        reorg_address,
        BlockHash::from(H256::random()),
        REORG_BLOCK_HEIGHT.into(),
        reorg_tx_hash,
        0,
        false,
    );
    let stale_log = log_generator.generate_log_with_info(EVENT_SIGNATURE, stale_log_info);
    let canonical_log = log_generator.generate_log_with_info(EVENT_SIGNATURE, canonical_log_info);
    // Reproduce the state a previous run would leave behind: the stale log saved, and a sync
    // checkpoint at the tip the run had reached.
    store.save_log(&stale_log)?;
    let checkpoint_log = log_generator.generate_log_with_info(
        EVENT_SIGNATURE,
        LogInfo::new(
            tip_address,
            BlockHash::from(H256::random()),
            CHECKPOINT_BLOCK_HEIGHT.into(),
            TxHash::from(H256::random()),
            0,
            false,
        ),
    );
    store.set_sync_checkpoint(&checkpoint_log)?;
    let mut mock_rsk_provider = MockRskProvider::new();
    // Recovery backs off from the checkpoint (30) by the finality depth (to 25) and re-fetches.
    // The reorged block now yields the canonical log; every other block returns nothing.
    let canonical_for_provider = canonical_log.clone();
    mock_rsk_provider.expect_get_logs().returning(move |from, to, _| {
        let reorg_block = BlockNumber::from(REORG_BLOCK_HEIGHT);
        if from <= reorg_block && reorg_block <= to {
            Ok(vec![canonical_for_provider.clone()])
        } else {
            Ok(vec![])
        }
    });
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    mock_rsk_provider_handler.set_provider_expect_get_block_by_hash(
        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
        INIT_BLOCK_HEIGHT.into(),
    );
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    // A single tip log delivered over the subscription so the run terminates cleanly.
    let tip_log_info = LogInfo::new(
        tip_address,
        BlockHash::from(H256::random()),
        MAX_BLOCK_HEIGHT.into(),
        TxHash::from(H256::random()),
        0,
        false,
    );
    let addresses = vec![reorg_address, tip_address];
    let filter =
        RskSubscriptionFilter::new(addresses.clone(), vec![], Some(MAX_BLOCK_HEIGHT.into()));
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(
        filter,
        EVENT_SIGNATURE.to_string(),
        vec![tip_log_info.clone()],
    );
    let managed_contracts = generate_fake_managed_contracts(addresses);
    cycle_indexer(
        store,
        mock_rsk_provider,
        &managed_contracts,
        &shutting_down,
        FINALITY_DEPTH,
        None,
    );
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    let log_key = format!("logs/{reorg_address}/{reorg_tx_hash}/0");
    let stored: RskLog = store_after.get(&log_key)?.expect("Reorged log not found in storage!");
    assert_eq!(
        stored, canonical_log,
        "Recovery should overwrite the stale log with the reorged one"
    );
    assert_ne!(stored, stale_log, "Stale (pre-reorg) log must not survive recovery");
    assert_logs(&log_generator, &store_after, EVENT_SIGNATURE, vec![tip_log_info]);
    Ok(())
}

/*
# Given the storage is empty
# And the provider delivers logs L1, L2, L3 under subscription
# When the log indexer runs and stores them (sync checkpoint ends at L3)
# And the indexer is restarted
# And the provider re-delivers the already-stored L3 followed by the new logs L4, L5
# Then the storage should contain L1..L5, with L3 stored exactly once (idempotent re-delivery)
# And the sync checkpoint should be L5
*/
#[test]
fn test_when_indexer_restarts_should_resume_from_checkpoint_without_duplicating_logs() -> Result<()>
{
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const LOG_COUNT: u64 = 5;
    const FIRST_LOG_BLOCK_HEIGHT: u64 = 26;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let log_generator = FakeLogGenerator::new();
    let addresses = generate_fake_addresses(LOG_COUNT);
    // Five distinct logs at consecutive blocks, built once so the shared log (L3) is byte-for-byte
    // identical across both runs.
    let logs: Vec<LogInfo> = (0..LOG_COUNT)
        .map(|i| {
            LogInfo::new(
                addresses[usize::try_from(i).expect("index too large for usize")],
                BlockHash::from(H256::random()),
                (FIRST_LOG_BLOCK_HEIGHT + i).into(),
                TxHash::from(H256::random()),
                0,
                false,
            )
        })
        .collect();
    let managed_contracts = generate_fake_managed_contracts(addresses.clone());

    // Phase 1: the first three logs are delivered and stored; the checkpoint ends at L3.
    run_subscription_phase(
        store_path,
        &addresses,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
        &managed_contracts,
        EVENT_SIGNATURE,
        vec![logs[0].clone(), logs[1].clone(), logs[2].clone()],
        Some("Phase 1 (initial sync) completed successfully."),
    )?;

    // Phase 2: the restart re-delivers the already-stored L3, then the new L4 and L5.
    run_subscription_phase(
        store_path,
        &addresses,
        MAX_BLOCK_HEIGHT_SUBSCRIPTION,
        &managed_contracts,
        EVENT_SIGNATURE,
        vec![logs[2].clone(), logs[3].clone(), logs[4].clone()],
        Some("Phase 2 (restart and resume) completed successfully."),
    )?;

    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    assert_logs(&log_generator, &store_after, EVENT_SIGNATURE, logs.clone());
    // L3 was delivered in both runs but must resolve to a single, correct entry.
    let l3 = &logs[2];
    let l3_key = format!("logs/{}/{}/0", l3.address(), l3.tx_hash());
    let stored_l3: RskLog = store_after.get(&l3_key)?.expect("L3 not found in storage!");
    assert_eq!(
        stored_l3,
        log_generator.generate_log_with_info(EVENT_SIGNATURE, l3.clone()),
        "Re-delivered log L3 should remain a single correct entry"
    );
    // The checkpoint should have advanced to the last delivered log, L5.
    let checkpoint = store_after.get_sync_checkpoint()?.expect("No sync checkpoint found");
    assert_eq!(
        checkpoint,
        log_generator.generate_log_with_info(EVENT_SIGNATURE, logs[4].clone()),
        "Sync checkpoint should be the last delivered log (L5)"
    );
    Ok(())
}

/// Runs one full indexer cycle (recovery with empty `get_logs` + subscription) over `store_path`,
/// delivering `log_info_tuples` through the subscription. Used to simulate indexer restarts.
fn run_subscription_phase(
    store_path: &str,
    addresses: &[Address],
    max_block_height: u64,
    managed_contracts: &HashMap<Address, ContractInfo>,
    event_signature: &str,
    log_info_tuples: Vec<LogInfo>,
    msg: Option<&str>,
) -> Result<()> {
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider = MockRskProvider::new();
    mock_rsk_provider.expect_get_logs().returning(|_, _, _| Ok(vec![]));
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        1.into(),
        max_block_height.into(),
        max_block_height.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
        None,
    );
    mock_rsk_provider_handler
        .set_provider_expect_get_block_by_hash(BlockHash::try_from(DEFAULT_BLOCK_HASH)?, 1.into());
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    let filter =
        RskSubscriptionFilter::new(addresses.to_vec(), vec![], Some(max_block_height.into()));
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(
        filter,
        event_signature.to_string(),
        log_info_tuples,
    );
    cycle_indexer(store, mock_rsk_provider, managed_contracts, &shutting_down, 0, msg);
    Ok(())
}

fn log_info_tuple_generator(
    filter_from_block_height: &Range<u64>,
    vec_size: u64,
    addresses: &[Address],
) -> Vec<LogInfo> {
    let mut v =
        Vec::with_capacity(usize::try_from(vec_size).expect("vec_size too large for usize"));
    let mut rng = rand::rng();
    let block_num_range = filter_from_block_height.clone();
    for i in 0..vec_size {
        let block_num = rng.random_range(block_num_range.clone());
        let address: Address = addresses[usize::try_from(i).expect("index too large for usize")];
        let block_hash = BlockHash::from(H256::random());
        let tx_hash = TxHash::from(H256::random());
        let log_index = rng.random_range(LOG_INDEX_RANGE);
        v.push(LogInfo::new(address, block_hash, block_num.into(), tx_hash, log_index, false));
    }
    v
}

fn cycle_indexer(
    store: RawLogStore,
    mock_rsk_provider: MockRskProvider,
    managed_contracts: &HashMap<Address, ContractInfo>,
    shutting_down: &ShutdownFlag,
    finality_depth: usize,
    msg: Option<&str>,
) {
    let indexer_config = IndexerConfig {
        start_from: IndexerStartFrom::Hash,
        initial_block_hash: Some(DEFAULT_BLOCK_HASH.to_string()),
        sync: common_runtime::config::SyncConfig { finality_depth, batch_size: 0 },
        storage: common_runtime::config::StorageConfig { path: String::new() },
        cache: common_runtime::config::CacheConfig { size: 0 },
    };

    let indexer = LogIndexer::new(
        store,
        mock_rsk_provider,
        &indexer_config,
        managed_contracts.clone(),
        shutting_down.clone(),
    )
    .context("Failed to create LogIndexer")
    .unwrap();
    let _ = indexer.run();
    info!("{}", msg.unwrap_or("Indexer run completed successfully."));
    drop(indexer);
}

fn assert_logs(
    log_generator: &FakeLogGenerator,
    store: &RawLogStore,
    event_signature: &str,
    log_info_tuples: Vec<LogInfo>,
) {
    for log_info in log_info_tuples {
        let expected_log = log_generator.generate_log_with_info(event_signature, log_info);
        let expected_log_key = format!(
            "logs/{}/{}/{}",
            expected_log.info().address(),
            expected_log.info().tx_hash(),
            expected_log.info().log_index()
        );
        let actual_log = store.get(&expected_log_key).unwrap().expect("Log not found in storage!");
        assert_eq!(expected_log, actual_log, "Log in storage does not match the expected log");
    }
}

fn assert_log_not_in_store(
    log_generator: &FakeLogGenerator,
    store: &RawLogStore,
    event_signature: &str,
    log_info: LogInfo,
) {
    let unexpected_log = log_generator.generate_log_with_info(event_signature, log_info);
    let unexpected_log_key = format!(
        "logs/{}/{}/{}",
        unexpected_log.info().address(),
        unexpected_log.info().tx_hash(),
        unexpected_log.info().log_index()
    );
    let actual_log: Option<RskLog> = store.get(&unexpected_log_key).unwrap();
    assert_eq!(actual_log, None, "Log should not be in storage");
}
