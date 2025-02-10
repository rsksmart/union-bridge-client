use crate::events::parse_event_to_json;
use crate::store::LogStore;
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::JsonAbi;
use alloy_primitives::bytes::Bytes;
use alloy_primitives::{LogData, B256};
use alloy_sol_types::{sol, SolEvent, SolType};
use anyhow::{anyhow, bail, Result};
use common::contracts::ContractInfo;
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::{RskProvider, RskProviderError};
use common::rsk_provider::{RskSubscription, RskSubscriptionFilter};
use common::shutdown_flag::ShutdownFlag;
use common::types::RskLog;
use log::{debug, error, info};

pub struct LogIndexer<P: RskProvider, S: LogStore> {
    _store: S,
    rsk_provider: P,
    _initial_block_hash: String,
    managed_contracts: Vec<ContractInfo>,
    shutdown_flag: ShutdownFlag,
}

// TODO(iago) Important! Reorgs!
impl<P: RskProvider, S: LogStore> LogIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: &str,
        managed_contracts: Vec<ContractInfo>,
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

        let contract_addresses = self
            .managed_contracts
            .iter()
            .map(|c| c.address.clone())
            .collect();

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

            // TODO(iago) if we go for the sol! approach, we don't need this
            let managed_contract = self
                .managed_contracts
                .iter()
                .find(|c| c.address.to_lowercase() == new_log.address.to_lowercase());

            if managed_contract.is_none() {
                error!("[subscribe_logs] Received unmanaged log: {:?}", new_log);
                continue;
            }

            let test = parse_event_to_json(new_log.topics, new_log.data)?;

            debug!("{:?}", test);
        }

        Ok(())
    }
}
