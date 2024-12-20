use crate::types::{RskBlock, RskLog, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::Header;
use anyhow::{anyhow, bail, Ok, Result};
use log::debug;
use serde_json::{json, Value};

pub trait RskWsProvider {
    type BlockSub: RskSubscription<String>;
    type LogsSub: RskSubscription<RskLog>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSub>;
    fn subscribe_logs(&self) -> Result<Self::LogsSub>;
    fn get_block_by_hash(&self, block_hash: &str) -> Result<RskBlock>;
    fn disconnect(self) -> Result<()>;
}

pub trait RskSubscription<T> {
    fn next(&mut self) -> Result<Option<T>>;
    fn unsubscribe(self) -> Result<()>;
}

pub struct AlloyRskWsProvider {
    provider: RootProvider<PubSubFrontend>,
    rt_sync: RuntimeSync,
}

impl AlloyRskWsProvider {
    // TODO call this logic from a "dependency injection" file
    pub fn new(url: &str) -> Result<AlloyRskWsProvider> {
        let rt_sync = RuntimeSync::new()?;
        let ws = WsConnect::new(url);
        let provider_setup = ProviderBuilder::new().on_ws(ws);
        let provider = rt_sync.run(provider_setup)?;
        Ok(AlloyRskWsProvider { provider, rt_sync })
    }
}

impl RskWsProvider for AlloyRskWsProvider {
    type BlockSub = AlloyBlockSubscription;
    type LogsSub = AlloyLogsSubscription;

    fn subscribe_blocks(&self) -> Result<Self::BlockSub> {
        let subscription_request = self.provider.subscribe_blocks();
        let sub = self.rt_sync.run(subscription_request)?;
        Ok(AlloyBlockSubscription::new(sub))
    }

    fn subscribe_logs(&self) -> Result<Self::LogsSub> {
        todo!()
    }

    fn get_block_by_hash(&self, block_hash: &str) -> Result<RskBlock> {
        let rpc_call = self
            .provider
            .client()
            .request("eth_getBlockByHash", vec![json!(block_hash), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;

        // TODO resilience when response is not a block (ie not found)

        let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
        let rsk_block: RskBlock = RskBlock::from(rpc_block);

        Ok(rsk_block)
    }

    fn disconnect(self) -> Result<()> {
        drop(self.provider);
        Ok(())
    }
}

pub struct AlloyBlockSubscription {
    sub: Subscription<Header>,
    rt_sync: RuntimeSync,
}

impl AlloyBlockSubscription {
    pub fn new(sub: Subscription<Header>) -> Self {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");
        AlloyBlockSubscription { sub, rt_sync }
    }
}

impl RskSubscription<String> for AlloyBlockSubscription {
    fn next(&mut self) -> Result<Option<String>> {
        let header = self.rt_sync.run(self.sub.recv_any())?;
        debug!("Received header: {:?}", header);

        let new_block_header_raw = match header {
            SubscriptionItem::Other(raw_json) => raw_json.get().to_string(),
            _ => {
                bail!("Unexpected SubscriptionItem: {:?}", header);
            }
        };

        let new_block_header: Value = serde_json::from_str(&*new_block_header_raw)?;
        let new_block_hash = new_block_header["hash"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing hash field"))?;

        Ok(Some(new_block_hash.to_string()))
    }

    fn unsubscribe(self) -> Result<()> {
        // Nothing to do apparently for this provider
        Ok(())
    }
}

pub struct AlloyLogsSubscription {}

impl RskSubscription<RskLog> for AlloyLogsSubscription {
    fn next(&mut self) -> Result<Option<RskLog>> {
        todo!()
    }

    fn unsubscribe(self) -> Result<()> {
        todo!()
    }
}
