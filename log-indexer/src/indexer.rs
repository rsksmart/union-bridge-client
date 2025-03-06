use crate::store::LogStore;
use anyhow::{bail, Context, Result};
use common::{
    rsk_indexer::RskIndexer,
    rsk_provider::{RskProvider, RskSubscription, RskSubscriptionError, RskSubscriptionFilter},
    shutdown_flag::ShutdownFlag,
    types::{BlockHash, BlockNumber, ContractInfo, RskLog},
};
use log::{error, info, warn};
use std::collections::HashMap;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    store: S,
    rsk_provider: P,
    initial_block_number: BlockNumber,
    managed_contracts: HashMap<String, ContractInfo>,
    shutdown_flag: ShutdownFlag,
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: BlockHash,
        managed_contracts: HashMap<String, ContractInfo>,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self> {
        let initial_block_number = provider
            .get_block_by_hash(initial_block_hash)
            .context("Failed to get initial block by hash")?
            .context("Initial block not found on provider")?
            .number();

        Ok(Self {
            store,
            rsk_provider: provider,
            initial_block_number,
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

        let contract_addresses: Vec<String> = self.managed_contracts.keys().cloned().collect();

        let best_block = self.rsk_provider.get_best_block()?;

        // TODO(Jira) Address this hardcoding in scope of https://rsklabs.atlassian.net/browse/UB-45
        let block_from = best_block.number() - 10;
        let filter = RskSubscriptionFilter::new(contract_addresses, vec![], Some(block_from));

        info!(
            "[subscribe_logs] Start subscribe_logs with filter {:?}...",
            filter
        );

        let mut rsk_log_subscription = self
            .rsk_provider
            .subscribe_logs(filter)
            .context("Failed to subscribe to logs")?; // do not retry, this is the application startup

        let loop_result = self.listen_logs(&mut rsk_log_subscription);

        // TODO(Jira) Implement shutdown/restart resilience (catch up) https://rsklabs.atlassian.net/browse/UB-45

        rsk_log_subscription
            .unsubscribe()
            .and_then(|_| loop_result)?;

        Ok(())
    }
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
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
                    error!("[subscribe_logs] Subscription lagged, a backward_sync will be needed: {err:?}");
                    continue;
                }
                Err(RskSubscriptionError::Unexpected(err)) => {
                    bail!("[subscribe_logs] Unknown error on log subs: {err:?}");
                }
            };

            if new_log.info().number() < self.initial_block_number {
                warn!(
                    "[subscribe_logs] Log block {} is lower than initial {}",
                    new_log.info().number(),
                    self.initial_block_number
                );
                continue;
            }

            info!("[subscribe_logs] Processed log: {:?}", new_log);

            let managed_contract = self
                .managed_contracts
                .get(&new_log.info().address().to_string());
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

            // TODO(Jira) send via broker after some configurable finality is achieved and taking into account `removed` field https://rsklabs.atlassian.net/browse/UB-46

            info!("Decoded event: {rsk_event:?}");
        }

        Ok(())
    }
}
