use crate::store::LogStore;
use anyhow::{Context, Result, bail};
use common::{
    rsk_indexer::RskIndexer,
    rsk_provider::{RskProvider, RskSubscription, RskSubscriptionError, RskSubscriptionFilter},
    shutdown_flag::ShutdownFlag,
    types::{Address, BlockHash, BlockNumber, ContractInfo, RskBlock, RskLog},
};
use log::{debug, error, info, warn};
use std::collections::HashMap;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    store: S,
    rsk_provider: P,
    initial_block_number: BlockNumber,
    sync_batch_size: usize,
    sync_finality_depth: usize,
    managed_contracts: HashMap<Address, ContractInfo>,
    shutdown_flag: ShutdownFlag,
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        rsk_provider: P,
        initial_block_hash: BlockHash,
        sync_batch_size: usize,
        sync_finality_depth: usize,
        managed_contracts: HashMap<Address, ContractInfo>,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self> {
        let initial_block_number = rsk_provider
            .get_block_by_hash(initial_block_hash)
            .context("Failed to get initial block by hash")?
            .context("Initial block not found on provider")?
            .number();

        Ok(Self {
            store,
            rsk_provider,
            initial_block_number,
            sync_batch_size,
            sync_finality_depth,
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

        let best_block = self.rsk_provider.get_best_block()?;

        let contract_addresses: Vec<Address> = self
            .managed_contracts
            .iter()
            .map(|c| c.1.address.clone())
            .collect();

        self.sync_logs(&best_block, &contract_addresses)?;

        let filter =
            RskSubscriptionFilter::new(contract_addresses, vec![], Some(best_block.number()));
        info!(
            "[subscribe_logs] Start subscribe_logs with filter {:?}...",
            filter
        );

        let mut rsk_log_subscription = self
            .rsk_provider
            .subscribe_logs(filter)
            .context("Failed to subscribe to logs")?; // do not retry, this is the application startup

        let loop_result = self.listen_logs(&mut rsk_log_subscription);

        rsk_log_subscription
            .unsubscribe()
            .and_then(|_| loop_result)?;

        Ok(())
    }
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    fn sync_logs(&self, best_block: &RskBlock, addrs: &Vec<Address>) -> Result<()> {
        let checkpoint = self.store.get_sync_checkpoint()?;

        // If there is no sync checkpoint present in storage,
        // use the initial block number present in config
        let mut start = match checkpoint {
            Some(log) => log.info().block_number(),
            None => self.initial_block_number,
        };
        let end = best_block.number();

        // This is needed in case there were previously logs saved in
        // storage that were later on reorganized
        let finality_depth = self.sync_finality_depth as u64;
        start = start - finality_depth;

        info!(
            "Starting logs sync from block {} to {}, finality depth: {}, batch size: {}",
            start, end, finality_depth, self.sync_batch_size
        );

        let batch_size = self.sync_batch_size as u64;
        while start <= end {
            let from = start;
            let to = if start + batch_size < end {
                start + batch_size
            } else {
                end
            };

            debug!("Fetching logs from block {} to {}", from, to);

            let logs = self.rsk_provider.get_logs(from, to, addrs)?;

            debug!("Fetched {} logs from {} to {}", logs.len(), from, to);

            logs.into_iter().try_for_each(|log| {
                debug!(
                    "Saving log at block {} tx {}",
                    log.info().block_number(),
                    log.info().tx_hash()
                );
                self.store.save_log(&log)
            })?;

            start = to + 1;
        }

        info!("Logs sync completed up to block {}", end);

        Ok(())
    }

    fn listen_logs(&self, rsk_log_subscription: &mut impl RskSubscription<RskLog>) -> Result<()> {
        while self.is_running() {
            let new_log = match rsk_log_subscription.next() {
                Ok(log) => log,
                Err(RskSubscriptionError::ClosedConnection) => {
                    if self.is_running() {
                        bail!("Provider closed unexpectedly!");
                    } else {
                        info!("[subscribe_logs] Shutdown requested, quitting...");
                        break;
                    }
                }
                Err(RskSubscriptionError::Transient(err)) => {
                    error!("[subscribe_logs] Ignoring problematic log: {err:?}");
                    continue;
                }
                Err(RskSubscriptionError::Lagged(err)) => {
                    // TODO(Jira) trigger backward sync in scope of https://rsklabs.atlassian.net/browse/UB-45
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

            info!("[subscribe_logs] Processed log: {:?}", new_log);

            let managed_contract = self.managed_contracts.get(&new_log.info().address());
            if managed_contract.is_none() {
                error!(
                    "[subscribe_logs] Received unmanaged contract log: {:?}",
                    new_log
                );
                continue;
            }
            let managed_contract = managed_contract.unwrap();

            let rsk_event_result = &self
                .rsk_provider
                .decode_log(new_log.clone(), managed_contract);

            let rsk_event = match rsk_event_result {
                Ok(Some(e)) => e,
                Ok(None) => {
                    error!("[subscribe_logs] Unmanaged log received: {:?}", new_log);
                    continue;
                }
                Err(e) => {
                    error!("[subscribe_logs] Ignoring malformed event: {:?}", e);
                    continue;
                }
            };

            self.store.save_log(&new_log).context("Saving new log")?;
            self.store
                .set_sync_checkpoint(&new_log)
                .context("Setting new log checkpoint")?;

            // TODO(Jira) send via broker after some configurable finality is achieved and taking into account `removed` field https://rsklabs.atlassian.net/browse/UB-46

            info!("Decoded event: {rsk_event:?}");
        }

        Ok(())
    }
}
