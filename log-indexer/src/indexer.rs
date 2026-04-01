use std::collections::HashMap;
use std::sync::mpsc;

use anyhow::{Context, Result, bail};
use common::config::{IndexerConfig, IndexerStartFrom};
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::{
    RskProvider, RskSubscription, RskSubscriptionError, RskSubscriptionFilter,
};
use common::shutdown_flag::ShutdownFlag;
use common::types::{Address, BlockNumber, ContractInfo, RskLog};
use log::{debug, error, info, trace, warn};

use crate::store::LogStore;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    store: S,
    rsk_provider: P,
    new_log_sender: Option<mpsc::Sender<RskLog>>,
    start_from: IndexerStartFrom,
    initial_block_number: BlockNumber,
    sync_batch_size: usize,
    sync_finality_depth: usize,
    managed_contracts: HashMap<Address, ContractInfo>,
    shutdown_flag: ShutdownFlag,
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    /// Create a new `LogIndexer` with a notifier channel
    ///
    /// # Errors
    ///
    /// Returns an error if the initial block cannot be retrieved from the provider
    pub fn new_with_notifier(
        store: S,
        rsk_provider: P,
        new_log_sender: mpsc::Sender<RskLog>,
        indexer_config: &IndexerConfig,
        managed_contracts: HashMap<Address, ContractInfo>,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self> {
        let initial_block = indexer_config.resolve_initial_block(&rsk_provider)?;

        Ok(Self {
            store,
            rsk_provider,
            new_log_sender: Some(new_log_sender),
            start_from: indexer_config.start_from,
            initial_block_number: initial_block.number(),
            sync_batch_size: indexer_config.sync.batch_size,
            sync_finality_depth: indexer_config.sync.finality_depth,
            managed_contracts,
            shutdown_flag,
        })
    }

    /// Create a new `LogIndexer`
    ///
    /// # Errors
    ///
    /// Returns an error if the initial block cannot be retrieved from the provider
    pub fn new(
        store: S,
        rsk_provider: P,
        indexer_config: &IndexerConfig,
        managed_contracts: HashMap<Address, ContractInfo>,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self> {
        let initial_block = indexer_config.resolve_initial_block(&rsk_provider)?;

        Ok(Self {
            store,
            rsk_provider,
            new_log_sender: None,
            start_from: indexer_config.start_from,
            initial_block_number: initial_block.number(),
            sync_batch_size: indexer_config.sync.batch_size,
            sync_finality_depth: indexer_config.sync.finality_depth,
            managed_contracts,
            shutdown_flag,
        })
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }
}

impl<P: RskProvider, S: LogStore> RskIndexer<P, S> for LogIndexer<P, S> {
    fn run(&self) -> Result<()> {
        if !self.is_running() {
            info!("[subscribe_logs] Shutdown requested, skipping...");
            return Ok(());
        }

        let contract_addresses: Vec<Address> =
            self.managed_contracts.iter().map(|c| c.1.address).collect();

        let last_block_number = match self.start_from {
            IndexerStartFrom::Best => {
                info!("[run] start_from='best': skipping historical log recovery");
                self.warn_if_filled_db()?;
                self.initial_block_number
            }
            IndexerStartFrom::Hash => {
                info!("[run] start_from='hash': recovering historical logs");
                self.recover_logs(&contract_addresses)?
            }
        };

        let filter =
            RskSubscriptionFilter::new(contract_addresses, vec![], Some(last_block_number));
        info!("[subscribe_logs] Start subscribe_logs with filter {filter:?}...");

        let mut rsk_log_subscription =
            self.rsk_provider.subscribe_logs(filter).context("Failed to subscribe to logs")?; // do not retry, this is the application startup

        let loop_result = self.listen_logs(&mut rsk_log_subscription);

        rsk_log_subscription.unsubscribe().and(loop_result)?;

        Ok(())
    }
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    fn warn_if_filled_db(&self) -> Result<()> {
        if let Some(checkpoint) = self.store.get_sync_checkpoint()? {
            warn!(
                "[subscribe_logs] Existing sync checkpoint found at block {}, tx {}, idx {}; start_from='best' ignores previous DB sync state and skips historical recovery",
                checkpoint.info().block_number(),
                checkpoint.info().tx_hash(),
                checkpoint.info().log_index(),
            );
        }
        Ok(())
    }

    fn recover_logs(&self, addrs: &[Address]) -> Result<BlockNumber> {
        let checkpoint = self.store.get_sync_checkpoint()?;
        let mut start = if let Some(log) = checkpoint {
            info!(
                "Resuming log sync from checkpoint at block {}, tx {}, idx {}",
                log.info().block_number(),
                log.info().tx_hash(),
                log.info().log_index()
            );
            log.info().block_number()
        } else {
            info!(
                "No sync checkpoint found, starting from initial block {}",
                self.initial_block_number
            );
            self.initial_block_number
        };

        // This is needed in case there were previously logs saved in
        // storage that were later on reorganized
        let finality_depth = self.sync_finality_depth as u64;
        let original_start = start;
        start = BlockNumber::from(start.value().saturating_sub(finality_depth));
        info!(
            "Adjusted start block for finality: original = {original_start}, finality_depth = {finality_depth}, adjusted = {start}"
        );

        let best_block = self.rsk_provider.get_best_block()?;
        let mut end = best_block.number();

        let mut attempt = 1;
        let max_attempts = 10;
        let batch_size = self.sync_batch_size as u64;

        info!(
            "[Attempt {attempt}/{max_attempts}] Starting logs sync from block {start} to {end} (batch size: {batch_size})"
        );

        while self.is_running() && start <= end {
            if attempt > max_attempts {
                bail!(
                    "Failed to recover logs after {max_attempts} attempts. Best block kept changing."
                );
            }

            let from = start;
            let to = if start + batch_size < end { start + batch_size } else { end };

            debug!("Fetching logs from block {from} to {to}");
            let logs = self.rsk_provider.get_logs(from, to, addrs)?;
            debug!("Fetched {} logs from {from} to {to}", logs.len());

            self.save_logs_and_checkpoint(&logs)?;

            if to == end {
                // Check if the best block has changed
                let new_best_block = self.rsk_provider.get_best_block()?;
                if end < new_best_block.number() {
                    info!(
                        "[Attempt {attempt}/{max_attempts}] New blocks appeared during sync: previous best = {end}, current best = {}. Continuing...",
                        new_best_block.number()
                    );
                    end = new_best_block.number();
                    attempt += 1;
                }
            }

            start = to + 1;
        }

        info!(
            "[Attempt {attempt}/{max_attempts}] Best block unchanged after sync (block {end}). Sync finished."
        );

        Ok(end)
    }

    fn save_logs_and_checkpoint(&self, logs: &[RskLog]) -> Result<()> {
        if logs.is_empty() {
            return Ok(());
        }

        let ids = logs
            .iter()
            .map(|log| {
                format!(
                    "[block: {}, tx: {}, idx: {}]",
                    log.info().block_number(),
                    log.info().tx_hash(),
                    log.info().log_index()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        debug!("Attempting to save {} logs: {}", logs.len(), ids);
        self.store.save_logs(logs)?;
        debug!("Successfully saved {} logs", logs.len());

        if let Some(last_log) = logs.last() {
            debug!(
                "Setting sync checkpoint at block {}, tx {}, idx {}",
                last_log.info().block_number(),
                last_log.info().tx_hash(),
                last_log.info().log_index()
            );
            self.store.set_sync_checkpoint(last_log)?;
        }

        Ok(())
    }

    fn listen_logs(&self, rsk_log_subscription: &mut impl RskSubscription<RskLog>) -> Result<()> {
        while self.is_running() {
            let new_log = match rsk_log_subscription.next() {
                Ok(log) => log,
                Err(RskSubscriptionError::ClosedConnection) => {
                    if self.is_running() {
                        bail!("Provider closed unexpectedly!");
                    }
                    info!("[subscribe_logs] Shutdown requested, quitting...");
                    break;
                }
                Err(RskSubscriptionError::Transient(err)) => {
                    error!("[subscribe_logs] Ignoring problematic log: {err:?}");
                    continue;
                }
                Err(RskSubscriptionError::Lagged(err)) => {
                    // TODO(UB-45) trigger backward sync
                    error!(
                        "[subscribe_logs] Subscription lagged, a backward_sync will be needed: {err:?}"
                    );
                    continue;
                }
                Err(RskSubscriptionError::Unexpected(err)) => {
                    bail!("[subscribe_logs] Unknown error on log subs: {err:?}");
                }
            };

            if new_log.info().block_number() < self.initial_block_number {
                warn!(
                    "[subscribe_logs] Log block {} is lower than initial {}",
                    new_log.info().block_number(),
                    self.initial_block_number
                );
                continue;
            }

            info!(
                "[subscribe_logs] Processed log {} @ {}",
                new_log.event().topics().first().map_or("none".to_string(), ToString::to_string),
                new_log.info().address(),
            );
            trace!("[subscribe_logs] Log: {new_log:?}");

            if !self.managed_contracts.contains_key(&new_log.info().address()) {
                error!(
                    "[subscribe_logs] Received unmanaged contract log {} [{:?}]",
                    new_log.info().address(),
                    self.managed_contracts
                );
                continue;
            }

            self.store.save_log(&new_log).context("Saving new log")?;
            // TODO(UB-111) avoid double writes for sync checkpoint in log indexer listener
            self.store.set_sync_checkpoint(&new_log).context("Setting new log checkpoint")?;

            #[allow(clippy::collapsible_if)]
            if let Some(channel) = &self.new_log_sender {
                if let Err(e) = channel.send(new_log) {
                    error!("Failed to send new block through channel: {e:?}");
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use common::rsk_provider::{MockRskProvider, MockRskSubscription, RskSubscriptionError};
    use common::types::*;
    use mockall::predicate::*;
    use primitive_types::{H160, H256, U256};

    use super::*;
    use crate::store::MockLogStore;
    const EMPTY_ADDRESSES: Vec<Address> = vec![];
    #[test]
    fn recover_logs_when_no_checkpoint_should_start_from_initial_block() {
        let mut mock_store = MockLogStore::new();
        let mut mock_provider = MockRskProvider::new();

        let finality_depth = 1;
        let best_block = block_with_number(100);
        let best_block_number = best_block.number();
        let initial_block: RskBlock = block_with_number(99);
        let log_from_initial_block = RskLog::new(
            LogInfo::new(
                Address::from(H160::random()),
                initial_block.hash(),
                initial_block.number(),
                TxHash::from(H256::random()),
                0,
                false,
            ),
            LogEvent::new(DataBytes("data".as_bytes().to_vec()), vec![]),
        );
        let log_clone_for_store = log_from_initial_block.clone();
        let log_clone_for_provider = log_from_initial_block.clone();

        mock_store.expect_get_sync_checkpoint().times(1).returning(|| Ok(None));

        mock_provider.expect_get_best_block().times(2).returning(move || Ok(best_block.clone()));

        mock_provider
            .expect_get_logs()
            .with(eq(initial_block.number() - finality_depth), eq(best_block_number), always())
            .times(1)
            .returning(move |_, _, _| Ok(vec![log_clone_for_provider.clone()]));

        mock_store
            .expect_save_logs()
            .with(eq(vec![log_clone_for_store.clone()]))
            .times(1)
            .returning(|_| Ok(()));

        mock_store
            .expect_set_sync_checkpoint()
            .with(eq(log_clone_for_store))
            .times(1)
            .returning(|_| Ok(()));

        let indexer = LogIndexer {
            store: mock_store,
            rsk_provider: mock_provider,
            new_log_sender: None,
            start_from: IndexerStartFrom::Hash,
            initial_block_number: BlockNumber::from(99),
            sync_batch_size: 10,
            sync_finality_depth: usize::try_from(finality_depth)
                .expect("finality_depth must be non-negative and fit in usize"),
            managed_contracts: HashMap::new(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let addresses = vec![];

        let result = indexer.recover_logs(&addresses);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), best_block_number);
    }

    #[test]
    fn recover_logs_when_checkpoint_exists_should_resume_from_checkpoint_block() {
        let mut mock_store = MockLogStore::new();
        let mut mock_provider = MockRskProvider::new();

        let finality_depth = 1;
        let checkpoint_block = block_with_number(123);
        let best_block = block_with_number(130);
        let best_block_number = best_block.number();
        let dummy_log = RskLog::new(
            LogInfo::new(
                Address::from(H160::random()),
                checkpoint_block.hash(),
                checkpoint_block.number(),
                TxHash::from(H256::random()),
                0,
                false,
            ),
            LogEvent::new(DataBytes::new("data".as_bytes().to_vec()), vec![]),
        );
        let log_clone_for_provider = dummy_log.clone();
        let log_clone_for_store = dummy_log.clone();

        mock_store
            .expect_get_sync_checkpoint()
            .times(1)
            .returning(move || Ok(Some(dummy_log.clone())));

        mock_provider.expect_get_best_block().times(2).returning(move || Ok(best_block.clone()));

        mock_provider
            .expect_get_logs()
            .with(eq(checkpoint_block.number() - finality_depth), eq(best_block_number), always())
            .times(1)
            .returning(move |_, _, _| Ok(vec![log_clone_for_provider.clone()]));

        mock_store
            .expect_save_logs()
            .with(eq(vec![log_clone_for_store.clone()]))
            .times(1)
            .returning(|_| Ok(()));

        mock_store
            .expect_set_sync_checkpoint()
            .with(eq(log_clone_for_store))
            .times(1)
            .returning(|_| Ok(()));

        let indexer = LogIndexer {
            store: mock_store,
            rsk_provider: mock_provider,
            new_log_sender: None,
            start_from: IndexerStartFrom::Hash,
            initial_block_number: BlockNumber::from(0), // should be ignored
            sync_batch_size: 10,
            sync_finality_depth: usize::try_from(finality_depth)
                .expect("finality_depth must be non-negative and fit in usize"),
            managed_contracts: HashMap::new(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let addresses = vec![];

        let result = indexer.recover_logs(&addresses);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), best_block_number);
    }

    #[test]
    fn recover_logs_should_continue_if_best_block_changes_after_sync() {
        let mut mock_store = MockLogStore::new();
        let mut mock_provider = MockRskProvider::new();

        mock_store.expect_get_sync_checkpoint().returning(|| Ok(None));

        let first_best = block_with_number(100);
        let second_best = block_with_number(105);
        let second_best_clone = second_best.clone();

        let mut call_count = 0;
        mock_provider.expect_get_best_block().times(3).returning(move || {
            if call_count == 0 {
                call_count += 1;
                Ok(first_best.clone())
            } else {
                call_count += 1;
                Ok(second_best_clone.clone())
            }
        });

        mock_provider.expect_get_logs().returning(|_, _, _| Ok(vec![]));

        let indexer = LogIndexer {
            store: mock_store,
            rsk_provider: mock_provider,
            new_log_sender: None,
            start_from: IndexerStartFrom::Hash,
            initial_block_number: BlockNumber::from(80),
            sync_batch_size: 10,
            sync_finality_depth: 0,
            managed_contracts: HashMap::new(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let result = indexer.recover_logs(&EMPTY_ADDRESSES);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), second_best.number());
    }

    #[test]
    fn recover_logs_should_fail_if_best_block_keeps_changing() {
        let mut mock_store = MockLogStore::new();
        let mut mock_provider = MockRskProvider::new();

        mock_store.expect_get_sync_checkpoint().returning(|| Ok(None));

        let mut counter = 100;
        mock_provider.expect_get_best_block().returning(move || {
            counter += 1;
            Ok(block_with_number(counter))
        });

        mock_provider.expect_get_logs().returning(|_, _, _| Ok(vec![]));

        let indexer = LogIndexer {
            store: mock_store,
            rsk_provider: mock_provider,
            new_log_sender: None,
            start_from: IndexerStartFrom::Hash,
            initial_block_number: BlockNumber::from(80),
            sync_batch_size: 10,
            sync_finality_depth: 0,
            managed_contracts: HashMap::new(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let result = indexer.recover_logs(&EMPTY_ADDRESSES);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to recover logs after"));
    }

    #[test]
    fn recover_logs_should_not_underflow_when_finality_depth_exceeds_start() {
        let mut mock_store = MockLogStore::new();
        let mut mock_provider = MockRskProvider::new();
        let best_block = block_with_number(2);

        mock_store.expect_get_sync_checkpoint().times(1).returning(|| Ok(None));

        mock_provider.expect_get_best_block().times(2).returning(move || Ok(best_block.clone()));

        mock_provider
            .expect_get_logs()
            .with(eq(BlockNumber::from(0)), eq(BlockNumber::from(2)), always())
            .times(1)
            .returning(|_, _, _| Ok(vec![]));

        let indexer = LogIndexer {
            store: mock_store,
            rsk_provider: mock_provider,
            new_log_sender: None,
            start_from: IndexerStartFrom::Hash,
            initial_block_number: BlockNumber::from(2),
            sync_batch_size: 10,
            sync_finality_depth: 10,
            managed_contracts: HashMap::new(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let result = indexer.recover_logs(&EMPTY_ADDRESSES);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), BlockNumber::from(2));
    }

    #[test]
    fn run_with_start_from_best_skips_historical_recovery() {
        let mut mock_store = MockLogStore::new();
        mock_store.expect_get_sync_checkpoint().times(1).returning(|| Ok(None));

        let mut mock_subscription = MockRskSubscription::new();
        let shutdown_flag = ShutdownFlag::init();
        let shutdown_for_next = shutdown_flag.clone();
        mock_subscription.expect_next().times(1).returning(move || {
            shutdown_for_next.set();
            Err(RskSubscriptionError::ClosedConnection)
        });
        mock_subscription.expect_unsubscribe().times(1).returning(|| Ok(()));

        let mut mock_provider = MockRskProvider::new();
        mock_provider.expect_get_best_block().never();
        mock_provider.expect_get_logs().never();
        mock_provider
            .expect_subscribe_logs()
            .withf(|filter| filter.from_block == Some(BlockNumber::from(100)))
            .times(1)
            .return_once(move |_| Ok(mock_subscription));

        let indexer = LogIndexer {
            store: mock_store,
            rsk_provider: mock_provider,
            new_log_sender: None,
            start_from: IndexerStartFrom::Best,
            initial_block_number: BlockNumber::from(100),
            sync_batch_size: 10,
            sync_finality_depth: 1,
            managed_contracts: HashMap::new(),
            shutdown_flag,
        };

        let result = indexer.run();
        assert!(result.is_ok());
    }

    fn block_with_number(n: u64) -> RskBlock {
        RskBlock::new(
            BlockNumber::from(n),
            BlockHash::from(H256::random()),
            BlockHash::from(H256::random()),
            BlockTimestamp::from(0),
            BlockDifficulty::from(U256::zero()),
            BlockDifficulty::from(U256::zero()),
            BlockPow::from(H256::random()),
            vec![],
        )
    }
}
