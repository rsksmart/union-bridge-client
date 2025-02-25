use crate::rpc::AlloyProvider;
use alloy_primitives::{Address, B256};
use alloy_pubsub::{Subscription, SubscriptionItem};
use alloy_rpc_types::{FilterBlockOption, Header, Log, Topic};
use anyhow::Result;
use anyhow::{anyhow, Context};
use common::rsk_provider::{
    RskProvider, RskSubscription, RskSubscriptionError, RskSubscriptionFilter,
};
use common::types::{LogEvent, LogInfo, RskBlock, RskLog};
use log::debug;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;

pub struct AlloySubscription<T> {
    subscription: Subscription<T>,
    provider: AlloyProvider,
}

impl<T: DeserializeOwned> AlloySubscription<T> {
    pub(super) fn next(&mut self) -> Result<SubscriptionItem<T>, RskSubscriptionError> {
        match self.subscription.blocking_recv_any() {
            Ok(item) => Ok(item),
            Err(RecvError::Closed) => Err(RskSubscriptionError::ClosedConnection),
            Err(RecvError::Lagged(n)) => Err(RskSubscriptionError::Lagged(n)),
        }
    }

    pub(super) fn unsubscribe(&self) -> Result<()> {
        self.provider
            .unsubscribe(std::any::type_name::<T>(), *self.subscription.local_id())
    }
}

impl AlloySubscription<Header> {
    pub(super) fn new(subscription: Subscription<Header>, provider: AlloyProvider) -> Self {
        AlloySubscription {
            subscription,
            provider,
        }
    }
}

impl RskSubscription<RskBlock> for AlloySubscription<Header> {
    fn next(&mut self) -> Result<RskBlock, RskSubscriptionError> {
        let header = self.next()?;

        debug!("Received header: {:?}", header);

        let new_block_header_raw = match header {
            SubscriptionItem::Other(raw_json) => raw_json.get().to_string(),
            _ => {
                return Err(RskSubscriptionError::Unexpected(anyhow!(
                    "Wrong Header: {:?}",
                    header
                )));
            }
        };

        let new_block_header: Value = serde_json::from_str(&*new_block_header_raw)
            .context(format!("Error parsing header json: {new_block_header_raw}",))
            .map_err(RskSubscriptionError::Unexpected)?;
        let new_block_hash = new_block_header["hash"].as_str().ok_or_else(|| {
            RskSubscriptionError::Unexpected(anyhow!(
                "Missing hash on header {:?}",
                new_block_header.to_string()
            ))
        })?;

        // TODO(Jira) tmp approach, try to get the required block data from the subscription itself (check Rsk and Alloy impl): https://rsklabs.atlassian.net/browse/UB-36
        let new_block = self
            .provider
            .get_block_by_hash(&new_block_hash)
            .context(format!(
                "Error getting block {new_block_hash} from Provider"
            ))
            .map_err(RskSubscriptionError::Unexpected)?;

        if new_block.is_none() {
            return Err(RskSubscriptionError::Unexpected(anyhow!(
                "hash {new_block_hash} informed by Provider does not exist"
            )));
        }

        Ok(new_block.unwrap())
    }

    fn unsubscribe(&self) -> Result<()> {
        self.unsubscribe()
    }
}

impl AlloySubscription<Log> {
    pub(super) fn new(subscription: Subscription<Log>, provider: AlloyProvider) -> Self {
        AlloySubscription {
            subscription,
            provider,
        }
    }

    pub(super) fn build_addresses(filter: &RskSubscriptionFilter) -> Result<Vec<Address>> {
        let addresses = filter
            .addresses
            .iter()
            .map(|addr| addr.parse::<Address>())
            .collect::<Result<Vec<Address>, _>>()
            .context("Could not parse filter addresses")?;

        Ok(addresses)
    }

    pub(super) fn build_block_option(filter: &RskSubscriptionFilter) -> FilterBlockOption {
        FilterBlockOption::Range {
            from_block: filter.from_block.map(|n| n.into()),
            to_block: None,
        }
    }

    pub(super) fn build_topics(
        filter: &RskSubscriptionFilter,
    ) -> Result<[Topic; 4], RskSubscriptionError> {
        let mut topics: [Topic; 4] = Default::default();

        for (i, t) in filter.topics.iter().take(4).enumerate() {
            let topic: Topic = Topic::from(
                t.parse::<B256>()
                    .context("Could not parse topic")
                    .map_err(RskSubscriptionError::Unexpected)?,
            );
            topics[i] = topic;
        }

        Ok(topics)
    }
}
impl RskSubscription<RskLog> for AlloySubscription<Log> {
    fn next(&mut self) -> Result<RskLog, RskSubscriptionError> {
        let log = self.next()?;

        debug!("Received log: {:?}", log);

        let new_log = match log {
            SubscriptionItem::Item(log) => log,
            _ => {
                return Err(RskSubscriptionError::Unexpected(anyhow!(
                    "Wrong Log: {:?}",
                    log
                )));
            }
        };

        let tx_hash = new_log
            .transaction_hash
            .map(|h| h.to_string())
            .ok_or_else(|| RskSubscriptionError::Transient("Missing transaction_hash"))?;

        let block_hash = new_log
            .block_hash
            .map(|h| h.to_string())
            .ok_or_else(|| RskSubscriptionError::Transient("Missing block_hash"))?;

        let block_number = new_log
            .block_number
            .ok_or_else(|| RskSubscriptionError::Transient("Missing transaction_hash"))?;

        let log_index = new_log
            .log_index
            .ok_or_else(|| RskSubscriptionError::Transient("Missing log index"))?;

        let log_info = LogInfo::new(
            new_log.address().to_string(),
            block_hash.clone(),
            block_number,
            tx_hash.clone(),
            log_index,
            new_log.removed,
        );

        let event_data = LogEvent::new(
            new_log.data().data.to_string(),
            new_log
                .data()
                .topics()
                .iter()
                .map(|t| t.to_string())
                .collect(),
        );

        Ok(RskLog::new(log_info, event_data))
    }

    fn unsubscribe(&self) -> Result<()> {
        self.unsubscribe()
    }
}
