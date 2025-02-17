use crate::store::LogStore;
use anyhow::{anyhow, bail, Result};
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::{RskProvider, RskProviderError};
use common::rsk_provider::{RskSubscription, RskSubscriptionFilter};
use common::shutdown_flag::ShutdownFlag;
use common::types::{ContractInfo, RskLog};
use log::{debug, error, info, warn};
use std::collections::HashMap;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    store: S,
    rsk_provider: P,
    initial_block_number: u64,
    managed_contracts: HashMap<String, ContractInfo>,
    shutdown_flag: ShutdownFlag,
}

impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: &str,
        managed_contracts: HashMap<String, ContractInfo>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        let initial_block_number = provider
            .get_block_by_hash(initial_block_hash)
            .expect("Failed to get initial block by hash")
            .expect("Initial block not found on provider")
            .number();

        Self {
            store,
            rsk_provider: provider,
            initial_block_number,
            managed_contracts,
            shutdown_flag,
        }
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

        let filter =
            RskSubscriptionFilter::new(contract_addresses, vec![], Some(best_block.number() - 10));

        info!(
            "[subscribe_logs] Start subscribe_logs with filter {:?}...",
            filter
        );

        let mut rsk_log_subscription = self
            .rsk_provider
            .subscribe_logs(filter)
            .expect("Failed to subscribe to logs (unrecoverable)"); // TODO(Jira) retry mechanism in scope of UB-15

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
                Err(RskProviderError::Closed) => {
                    if self.is_running() {
                        bail!("[subscribe_logs] Provider closed unexpectedly");
                    } else {
                        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
                        info!("[subscribe_logs] Shutdown requested, quitting...");
                        break;
                    }
                }
                Err(e) => {
                    return Err(anyhow!("Failed to get next log from subscription: {:?}", e));
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

            debug!("[subscribe_logs] Processed log: {:?}", new_log);

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

            let rsk_event = &self
                .rsk_provider
                .decode_log(new_log.clone(), managed_contract.unwrap())?;

            if rsk_event.is_none() {
                error!("[subscribe_logs] Unmanaged log received: {:?}", new_log);
                continue;
            }

            self.store.save_log(&new_log)?;

            // TODO(Jira) send via broker after some configurable finality is achieved and taking into account `removed` field https://rsklabs.atlassian.net/browse/UB-46

            info!("Decoded event: {}", serde_json::to_string(&rsk_event)?);
        }

        Ok(())
    }
}
