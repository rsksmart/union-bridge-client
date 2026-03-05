use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use common::config::{IndexerConfig, IndexerStartFrom};
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::{MockRskProvider, RskSubscriptionFilter};
use common::shutdown_flag::ShutdownFlag;
use common::test_utils::mock_rsk_provider_handler::MockRskProviderHandler;
use common::test_utils::rsk_block_generator::FakeBlockGenerator;
use common::test_utils::rsk_log_generator::FakeLogGenerator;
use common::test_utils::rsk_utils::{
    DEFAULT_BLOCK_HASH, generate_fake_address, generate_fake_addresses,
    generate_fake_managed_contracts,
};
use common::types::{Address, BlockHash, ContractInfo, LogInfo, RskLog, TxHash};
use log::info;
use log_indexer::indexer::LogIndexer;
use log_indexer::store::RawLogStore;
use primitive_types::H256;
use rand::Rng;
use tempfile::tempdir;

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
    let _ = env_logger::builder().is_test(true).try_init();
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
    cycle_indexer(store, mock_rsk_provider, &managed_contracts, &shutting_down, None);
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
    let _ = env_logger::builder().is_test(true).try_init();
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
    cycle_indexer(store, mock_rsk_provider, &managed_contracts, &shutting_down, None);
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    assert_logs(&log_generator, &store_after, EVENT_SIGNATURE, log_info_tuples);
    assert_log_not_in_store(&log_generator, &store_after, EVENT_SIGNATURE, bad_log_info);
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
    msg: Option<&str>,
) {
    let indexer_config = IndexerConfig {
        start_from: IndexerStartFrom::Hash,
        initial_block_hash: Some(DEFAULT_BLOCK_HASH.to_string()),
        sync: common::config::SyncConfig { finality_depth: 0, batch_size: 0 },
        storage: common::config::StorageConfig { path: String::new() },
        cache: common::config::CacheConfig { size: 0 },
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
