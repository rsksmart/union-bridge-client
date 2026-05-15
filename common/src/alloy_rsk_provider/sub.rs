use alloy_primitives::{Address as AlloyAddress, B256};
use alloy_pubsub::{Subscription, SubscriptionItem};
use alloy_rpc_types::{FilterBlockOption, Header, Log, Topic};
use anyhow::{Context, Result, anyhow};
use log::trace;
use serde::de::DeserializeOwned;
use tokio::sync::broadcast::error::RecvError;

use crate::alloy_rsk_provider::rpc::AlloyProvider;
use crate::rsk_provider::{
    RskProvider, RskSubscription, RskSubscriptionError, RskSubscriptionFilter,
};
use crate::types::{
    Address, BlockHash, BlockNumber, DataBytes, LogEvent, LogInfo, LogTopic, RskBlock, RskLog,
    TxHash,
};

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
        self.provider.unsubscribe(std::any::type_name::<T>(), *self.subscription.local_id())
    }
}

impl AlloySubscription<Header> {
    pub(super) fn new(subscription: Subscription<Header>, provider: AlloyProvider) -> Self {
        AlloySubscription { subscription, provider }
    }

    #[cfg(feature = "anvil")]
    fn get_block_hash(header: SubscriptionItem<Header>) -> Result<BlockHash, RskSubscriptionError> {
        match header {
            SubscriptionItem::Item(h) => {
                Ok(BlockHash::try_from(h.hash.to_string().as_str()).expect("valid hash"))
            }
            SubscriptionItem::Other(_) => {
                Err(RskSubscriptionError::Unexpected(anyhow!("Wrong Header: {header:?}")))
            }
        }
    }

    #[cfg(not(feature = "anvil"))]
    fn get_block_hash(header: SubscriptionItem<Header>) -> Result<BlockHash, RskSubscriptionError> {
        use serde_json::Value;

        let new_block_header_raw = match header {
            SubscriptionItem::Item(_) => {
                return Err(RskSubscriptionError::Unexpected(anyhow!(
                    "Expected raw JSON header, got Item variant: {header:?}"
                )));
            }
            SubscriptionItem::Other(raw_json) => raw_json.get().to_string(),
        };

        let new_block_header: Value = serde_json::from_str(&new_block_header_raw)
            .context(format!("Error parsing header json: {new_block_header_raw}"))
            .map_err(RskSubscriptionError::Unexpected)?;
        let new_block_hash = new_block_header["hash"].as_str().ok_or_else(|| {
            RskSubscriptionError::Unexpected(anyhow!(
                "Missing hash on header {:?}",
                new_block_header.to_string()
            ))
        })?;
        let new_block_hash = BlockHash::try_from(new_block_hash)
            .map_err(|err| RskSubscriptionError::Unexpected(anyhow!(err)))?;

        Ok(new_block_hash)
    }
}

impl RskSubscription<RskBlock> for AlloySubscription<Header> {
    fn next(&mut self) -> Result<RskBlock, RskSubscriptionError> {
        let header = self.next()?;

        trace!("Received header: {header:?}");

        let new_block_hash = Self::get_block_hash(header)?;

        let new_block = self
            .provider
            .get_block_by_hash(new_block_hash)
            .context(format!("Error getting block {new_block_hash} from Provider"))
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
        AlloySubscription { subscription, provider }
    }

    pub(super) fn build_addresses(filter: &RskSubscriptionFilter) -> Result<Vec<AlloyAddress>> {
        let addresses = filter
            .addresses
            .iter()
            .map(|addr| {
                addr.to_string().parse::<AlloyAddress>().context("Parsing to Address failed")
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(addresses)
    }

    pub(super) fn build_block_option(filter: &RskSubscriptionFilter) -> FilterBlockOption {
        FilterBlockOption::Range {
            from_block: filter.from_block.map(|n| n.value().into()),
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

        trace!("Received log: {log:?}");

        let SubscriptionItem::Item(new_log) = log else {
            return Err(RskSubscriptionError::Unexpected(anyhow!("Wrong Log: {log:?}")));
        };

        let tx_hash = new_log
            .transaction_hash
            .map(TxHash::try_from)
            .ok_or_else(|| RskSubscriptionError::Transient("Missing transaction_hash"))?
            .map_err(|e| RskSubscriptionError::Unexpected(e.into()))?;

        let block_hash = new_log
            .block_hash
            .map(|h| BlockHash::try_from(h.to_string().as_str()))
            .ok_or_else(|| RskSubscriptionError::Transient("Missing block_hash"))?
            .map_err(|e| RskSubscriptionError::Unexpected(e.into()))?;

        let block_number = new_log
            .block_number
            .map(BlockNumber::from)
            .ok_or_else(|| RskSubscriptionError::Transient("Missing transaction_hash"))?;

        let log_index = new_log
            .log_index
            .ok_or_else(|| RskSubscriptionError::Transient("Missing log_index"))?;

        let address = Address::try_from(new_log.address().to_string().as_str())
            .map_err(|e| RskSubscriptionError::Unexpected(e.into()))?;

        let log_info =
            LogInfo::new(address, block_hash, block_number, tx_hash, log_index, new_log.removed);

        let event_data = LogEvent::new(
            DataBytes::new(new_log.data().data.to_vec()),
            new_log.data().topics().iter().map(|t| LogTopic::from(*t)).collect(),
        );

        Ok(RskLog::new(log_info, event_data))
    }

    fn unsubscribe(&self) -> Result<()> {
        self.unsubscribe()
    }
}
