use crate::event_processor::managed_contracts::ContractInfo;
use crate::event_processor::{event_processor_abi, event_processor_typed};
use crate::store::LogStore;
use anyhow::{anyhow, bail, Result};
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::{RskProvider, RskProviderError};
use common::rsk_provider::{RskSubscription, RskSubscriptionFilter};
use common::shutdown_flag::ShutdownFlag;
use common::types::RskLog;
use log::{debug, error, info};
use serde::Serialize;
use std::collections::HashMap;

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    _store: S,
    rsk_provider: P,
    _initial_block_hash: String,
    managed_contracts: HashMap<String, ContractInfo>,
    shutdown_flag: ShutdownFlag,
}

// TODO(iago) Important! Reorgs!
impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: &str,
        managed_contracts: HashMap<String, ContractInfo>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            _store: store,
            rsk_provider: provider,
            _initial_block_hash: initial_block_hash.to_string(),
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

        // TODO(iago) pass a range and filter out already known, otherwise on restart we receive bunch (not sure how the node decides which ones to provide us)
        let filter = RskSubscriptionFilter::new_logs_by_address(contract_addresses);

        info!(
            "[subscribe_logs] Start subscribe_logs with filter {:?}...",
            filter
        );

        let mut rsk_log_subscription = self
            .rsk_provider
            .subscribe_logs(filter)
            .expect("Failed to subscribe to logs (unrecoverable)"); // TODO retry mechanism in scope of UB-15

        let loop_result = self.listen_logs(&mut rsk_log_subscription);

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

            info!("[subscribe_logs] Processed log: {:?}", new_log);

            let managed_contract = self.managed_contracts.get(&new_log.address);
            if managed_contract.is_none() {
                error!(
                    "[subscribe_logs] Received unmanaged contract log: {:?}",
                    new_log
                );
                continue;
            }

            let log_as_json = Self::process_log(&new_log, managed_contract.unwrap())?;

            debug!(
                "Processed log, event: {}",
                serde_json::to_string(&log_as_json)?
            );
        }

        Ok(())
    }

    fn process_log(
        new_log: &RskLog,
        managed_contract: &ContractInfo,
    ) -> Result<Option<impl Serialize>> {
        // TODO(iago) get from env
        let dynamic_processing = true;
        if dynamic_processing {
            match event_processor_abi::process(&new_log, managed_contract)? {
                Some(e) => Ok(Some(serde_json::to_value(e)?)),
                None => Ok(None),
            }
        } else {
            match event_processor_typed::process(&new_log)? {
                Some(e) => Ok(Some(serde_json::to_value(e)?)),
                None => Ok(None),
            }
        }
    }
}
