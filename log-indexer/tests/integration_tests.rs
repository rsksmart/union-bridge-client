use rand::Rng;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use anyhow::{Context, Result};
use common::{
    rsk_indexer::RskIndexer,
    rsk_provider::{MockRskProvider, RskSubscriptionFilter},
    shutdown_flag::ShutdownFlag,
    types::{BlockHash, ContractInfo},
};
use log::info;
use log_indexer::{indexer::LogIndexer, store::RawLogStore};
use tempfile::tempdir;
use test_utils::{
    mock_rsk_provider_handler::MockRskProviderHandler,
    rsk_block_generator::FakeBlockGenerator,
    rsk_log_generator::FakeLogGenerator,
    rsk_utils::{generate_fake_addresses, generate_fake_managed_contracts, DEFAULT_BLOCK_HASH},
};

const FILTER_BLOCK_FROM_DEPTH: u64 = 10;
const TX_ID_RANGE: Range<u64> = 0..20;
const ADDRESSES_SIZE: u64 = 10;
const LOG_INDEX_RANGE: Range<u64> = 0..20;

/*
# Given the storage is empty
# And the provider retrieves logs B to L under subscription (B < L)
# When the log indexer is started
# Then the storage should contain logs from B to L
*/
#[test]
fn test_when_log_indexer_runs_should_add_logs_from_subscription() -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    const INIT_BLOCK_HEIGHT: u64 = 1;
    const MAX_BLOCK_HEIGHT_SUBSCRIPTION: u64 = 35;
    const LOG_BLOCK_HEIGHT_RANGE: Range<u64> =
        MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH..MAX_BLOCK_HEIGHT_SUBSCRIPTION;
    const LOG_VEC_TUPLE_SIZE: u64 = 10;
    const DELAY_BETWEEN_BLOCKS_SUBSCRIPTION: u64 = 2;

    let temp_dir = tempdir()?;
    let store_path = temp_dir.path().to_str().unwrap();
    let store = RawLogStore::new(store_path)?;

    let event_signature = "Transfer(address,address,uint256)";
    let block_generator = FakeBlockGenerator::new(0.into(), Arc::new(AtomicBool::new(false)));
    let log_generator = FakeLogGenerator::new(event_signature);
    let shutting_down = ShutdownFlag::init();
    let mock_rsk_provider = Arc::new(Mutex::new(MockRskProvider::new()));

    let mut mock_rsk_provider_handler = MockRskProviderHandler::new(
        Arc::clone(&mock_rsk_provider),
        &block_generator,
        Some(&log_generator),
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
    let addresses: Vec<String> = generate_fake_addresses(ADDRESSES_SIZE);
    let log_vec_tuple: Vec<(u64, u64, String, u64)> = log_vec_tuple_generator(
        LOG_BLOCK_HEIGHT_RANGE,
        LOG_VEC_TUPLE_SIZE,
        addresses.clone(),
    );

    let filter = RskSubscriptionFilter::new(
        addresses.clone(),
        vec![],
        Some((MAX_BLOCK_HEIGHT_SUBSCRIPTION - FILTER_BLOCK_FROM_DEPTH).into()),
    );
    mock_rsk_provider_handler.set_provider_expect_subscribe_logs(filter, log_vec_tuple.clone());
    mock_rsk_provider_handler.set_provider_expect_decode_log();
    drop(mock_rsk_provider_handler);
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
        &block_generator,
        &store_after,
        log_vec_tuple,
    );
    Ok(())
}

fn log_vec_tuple_generator(
    filter_from_block_height: Range<u64>,
    log_vec_tuple_size: u64,
    addresses: Vec<String>,
) -> Vec<(u64, u64, String, u64)> {
    let mut v = Vec::with_capacity(log_vec_tuple_size as usize);
    let mut rng = rand::rng();
    let block_num_range = filter_from_block_height.clone();
    for i in 0..log_vec_tuple_size {
        let block_num = rng.random_range(block_num_range.clone());
        let tx_id = rng.random_range(TX_ID_RANGE);
        let address: String = addresses[i as usize].clone();
        let log_index = rng.random_range(LOG_INDEX_RANGE);
        v.push((block_num, tx_id, address, log_index));
    }
    v
}

fn cycle_indexer(
    store: RawLogStore,
    mock_rsk_provider: Arc<Mutex<MockRskProvider>>,
    managed_contracts: HashMap<String, ContractInfo>,
    shutting_down: ShutdownFlag,
    msg: Option<&str>,
) -> () {
    let mock_rsk_provider = Arc::try_unwrap(mock_rsk_provider)
        .unwrap()
        .into_inner()
        .unwrap();
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
    block_generator: &FakeBlockGenerator,
    store: &RawLogStore,
    log_tuples: Vec<(u64, u64, String, u64)>,
) -> () {
    for (block_num, tx_id, address, log_index) in log_tuples {
        let block = block_generator.generate_block(block_num.into());
        let expected_log = log_generator.generate_log(block, tx_id, address, log_index);
        let expected_log_key = format!(
            "logs/{}/{}/{}",
            expected_log.info().address().to_string(),
            expected_log.info().tx_hash().to_string(),
            expected_log.info().log_index()
        );
        let actual_log = store.get(expected_log_key).unwrap().unwrap();
        assert_eq!(
            expected_log, actual_log,
            "Log in storage does not match the expected log"
        );
    }
}
