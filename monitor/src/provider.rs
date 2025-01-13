use crate::types::{RskBlock, RskLog, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::Header;
use anyhow::{anyhow, bail, Result};
use log::{debug, trace};
use serde_json::{json, Value};
use std::sync::Arc;

pub trait RskWsProvider {
    type BlockSub: RskSubscription<String>;
    type LogsSub: RskSubscription<RskLog>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSub>;
    fn subscribe_logs(&self) -> Result<Self::LogsSub>;
    fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock>;
    fn get_block_by_number(&self, num: u64) -> Result<RskBlock>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn disconnect(self) -> Result<()>;
}

pub trait RskSubscription<T> {
    fn next(&mut self) -> Result<Option<T>>;
    fn unsubscribe(self) -> Result<()>;
}

pub struct AlloyRskWsProvider {
    provider: RootProvider<PubSubFrontend>,
    rt_sync: Arc<RuntimeSync>,
}

impl AlloyRskWsProvider {
    // TODO(iago) call this logic from a "dependency injection" file
    pub fn new(url: &str, rt_sync: Arc<RuntimeSync>) -> Result<Self> {
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
        Ok(AlloyBlockSubscription::new(sub, self.rt_sync.clone()))
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

        // TODO(iago) resilience when response is not a block (ie not found)

        let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
        let rsk_block: RskBlock = RskBlock::from(rpc_block);

        Ok(rsk_block)
    }

    fn get_block_by_number(&self, num: u64) -> Result<RskBlock> {
        let num_hex = format!("0x{:x}", num);

        let rpc_call = self
            .provider
            .client()
            .request("eth_getBlockByNumber", vec![json!(num_hex), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;

        // TODO(iago) resilience when response is not a block (ie not found)

        let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
        let rsk_block: RskBlock = RskBlock::from(rpc_block);

        Ok(rsk_block)
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        let rpc_call = self.provider.client().request_noparams("eth_blockNumber");
        let response: Value = self.rt_sync.run(rpc_call)?;
        let number_hex: String = serde_json::from_value(response)?;
        let number_dec = u64::from_str_radix(&number_hex.trim_start_matches("0x"), 16)?;
        self.get_block_by_number(number_dec)
    }

    fn disconnect(self) -> Result<()> {
        drop(self.provider);
        Ok(())
    }
}

pub struct AlloyBlockSubscription {
    sub: Subscription<Header>,
    rt_sync: Arc<RuntimeSync>,
}

impl AlloyBlockSubscription {
    pub fn new(sub: Subscription<Header>, rt_sync: Arc<RuntimeSync>) -> Self {
        Self { sub, rt_sync }
    }
}

impl RskSubscription<String> for AlloyBlockSubscription {
    fn next(&mut self) -> Result<Option<String>> {
        let header = match self.sub.try_recv_any() {
            Ok(header) => header,
            Err(_) => {
                trace!("No new block yet");
                return Ok(None);
            }
        };

        trace!("Received header: {:?}", header);

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
