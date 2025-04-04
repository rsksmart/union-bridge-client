#![cfg(feature = "testing")]

use anyhow::{Context, Result};
use common::{
    rsk_indexer::RskIndexer,
    rsk_provider::{MockRskProvider, RskSubscriptionFilter},
    shutdown_flag::ShutdownFlag,
    types::{Address, BlockHash, ContractInfo, LogInfo},
};
use log::info;
use log_indexer::{indexer::LogIndexer, store::RawLogStore};
use primitive_types::H256;
use rand::Rng;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{atomic::AtomicBool, Arc},
};
use tempfile::tempdir;
use test_utils::{
    mock_rsk_provider_handler::MockRskProviderHandler,
    rsk_block_generator::FakeBlockGenerator,
    rsk_log_generator::FakeLogGenerator,
    rsk_utils::{
        generate_fake_address, generate_fake_addresses, generate_fake_managed_contracts,
        generate_fake_tx_hash, DEFAULT_BLOCK_HASH,
    },
};

const TX_ID_RANGE: Range<u64> = 0..20;
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
    let _ = env_logger::builder().is_test(true).try_init();
    const LOG_INFO_TUPLE_SIZE: u64 = 10;
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const FILTER_BLOCK_FROM_DEPTH: u64 = 10;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const LOG_BLOCK_HEIGHT_RANGE: Range<u64> =
        MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH..MAX_BLOCK_HEIGHT_SUBSCRIPTION;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
    let log_generator = FakeLogGenerator::new();
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider = MockRskProvider::new();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    mock_rsk_provider_handler.set_provider_expect_get_block_by_hash(
        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
        INIT_BLOCK_HEIGHT.into(),
    );
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    let addresses: Vec<Address> = generate_fake_addresses(LOG_INFO_TUPLE_SIZE);
    let log_info_tuples: Vec<LogInfo> = log_info_tuple_generator(
        LOG_BLOCK_HEIGHT_RANGE,
        LOG_INFO_TUPLE_SIZE,
        addresses.clone(),
    );
    let filter = RskSubscriptionFilter::new(
        addresses.clone(),
        vec![],
        Some((MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH).into()),
    );
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(
        filter,
        EVENT_SIGNATURE.to_string(),
        log_info_tuples.clone(),
    );
    mock_rsk_provider_handler.set_provider_expect_decode_log();
    let managed_contracts = generate_fake_managed_contracts(addresses);
    cycle_indexer(
        store,
        mock_rsk_provider,
        managed_contracts,
        shutting_down,
        None,
    );
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    assert_logs(
        &log_generator,
        &store_after,
        EVENT_SIGNATURE,
        log_info_tuples,
    );
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
    let _ = env_logger::builder().is_test(true).try_init();
    const LOG_INFO_TUPLE_SIZE: u64 = 10;
    const EVENT_SIGNATURE: &str = "Transfer(address,address,uint256)";
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const FILTER_BLOCK_FROM_DEPTH: u64 = 10;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const LOG_BLOCK_HEIGHT_RANGE: Range<u64> =
        MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH..MAX_BLOCK_HEIGHT_SUBSCRIPTION;
    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;
    let block_generator = FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
    let log_generator = FakeLogGenerator::new();
    let shutting_down = ShutdownFlag::init();
    let mut mock_rsk_provider = MockRskProvider::new();
    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        &mut mock_rsk_provider,
        &block_generator,
        Arc::new(AtomicBool::new(false)),
        shutting_down.clone(),
        INIT_BLOCK_HEIGHT.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        MAX_BLOCK_HEIGHT_SUBSCRIPTION.into(),
        DELAY_BETWEEN_BLOCKS_SUBSCRIPTION,
    );
    mock_rsk_provider_handler.set_provider_expect_get_block_by_hash(
        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
        INIT_BLOCK_HEIGHT.into(),
    );
    mock_rsk_provider_handler.set_provider_expect_get_best_block();
    let addresses: Vec<Address> = generate_fake_addresses(LOG_INFO_TUPLE_SIZE);
    let log_info_tuples: Vec<LogInfo> = log_info_tuple_generator(
        LOG_BLOCK_HEIGHT_RANGE,
        LOG_INFO_TUPLE_SIZE,
        addresses.clone(),
    );
    let filter = RskSubscriptionFilter::new(
        addresses.clone(),
        vec![],
        Some((MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH).into()),
    );
    let bad_log_info = LogInfo::new(
        generate_fake_address(LOG_INFO_TUPLE_SIZE + 1),
        BlockHash::from(H256::random()),
        (INIT_BLOCK_HEIGHT - 1).into(),
        generate_fake_tx_hash(1, ""),
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
    mock_rsk_provider_handler.set_provider_expect_decode_log();
    let managed_contracts = generate_fake_managed_contracts(addresses);
    cycle_indexer(
        store,
        mock_rsk_provider,
        managed_contracts,
        shutting_down,
        None,
    );
    let store_after: RawLogStore = RawLogStore::new(store_path)?;
    assert_logs(
        &log_generator,
        &store_after,
        EVENT_SIGNATURE,
        log_info_tuples,
    );
    assert_log_not_in_store(&log_generator, &store_after, EVENT_SIGNATURE, bad_log_info);
    Ok(())
}

fn log_info_tuple_generator(
    filter_from_block_height: Range<u64>,
    vec_size: u64,
    addresses: Vec<Address>,
) -> Vec<LogInfo> {
    let mut v = Vec::with_capacity(vec_size as usize);
    let mut rng = rand::rng();
    let block_num_range = filter_from_block_height.clone();
    for i in 0..vec_size {
        let block_num = rng.random_range(block_num_range.clone());
        let tx_id = rng.random_range(TX_ID_RANGE);
        let address: Address = addresses[i as usize].clone();
        let block_hash = BlockHash::from(H256::random());
        let tx_hash = generate_fake_tx_hash(tx_id, "");
        let log_index = rng.random_range(LOG_INDEX_RANGE);
        v.push(LogInfo::new(
            address,
            block_hash,
            block_num.into(),
            tx_hash,
            log_index,
            false,
        ));
    }
    v
}

fn cycle_indexer(
    store: RawLogStore,
    mock_rsk_provider: MockRskProvider,
    managed_contracts: HashMap<String, ContractInfo>,
    shutting_down: ShutdownFlag,
    msg: Option<&str>,
) -> () {
    let indexer = LogIndexer::new(
        store,
        mock_rsk_provider,
        BlockHash::try_from(DEFAULT_BLOCK_HASH).unwrap(),
        managed_contracts,
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
) -> () {
    for log_info in log_info_tuples {
        let expected_log = log_generator.generate_log(event_signature, log_info);
        let expected_log_key = format!(
            "logs/{}/{}/{}",
            expected_log.info().address().to_string(),
            expected_log.info().tx_hash().to_string(),
            expected_log.info().log_index()
        );
        let actual_log = store
            .get(expected_log_key)
            .unwrap()
            .expect("Log not found in storage!");
        assert_eq!(
            expected_log, actual_log,
            "Log in storage does not match the expected log"
        );
    }
}

fn assert_log_not_in_store(
    log_generator: &FakeLogGenerator,
    store: &RawLogStore,
    event_signature: &str,
    log_info: LogInfo,
) -> () {
    let unexpected_log = log_generator.generate_log(event_signature, log_info);
    let unexpected_log_key = format!(
        "logs/{}/{}/{}",
        unexpected_log.info().address().to_string(),
        unexpected_log.info().tx_hash().to_string(),
        unexpected_log.info().log_index()
    );
    let actual_log = store.get(unexpected_log_key).unwrap();
    assert_eq!(actual_log, None, "Log should not be in storage");
}
