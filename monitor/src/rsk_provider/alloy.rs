use crate::rsk_provider::provider::{RskBlockSubscription, RskProvider};
use crate::types::{RskBlock, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::Header;
use anyhow::{anyhow, bail, Result};
use log::trace;
use serde_json::{json, Value};
use std::sync::Arc;
// TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15

struct AlloyBlockSubscription {
    subscription: Subscription<Header>,
    provider: AlloyProvider,
}

impl AlloyBlockSubscription {
    fn new(provider: AlloyProvider) -> Result<Self> {
        let subscription_request = provider.provider.subscribe_blocks();
        let subscription = provider.rt_sync.run(subscription_request)?;
        Ok(AlloyBlockSubscription {
            subscription,
            provider,
        })
    }
}

impl RskBlockSubscription for AlloyBlockSubscription {
    fn next(&mut self) -> Result<Option<RskBlock>> {
        let header = match self.subscription.try_recv_any() {
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

        // TODO(iago) resilience when block is not found
        Ok(Some(self.provider.get_block_by_hash(&new_block_hash)?))
    }

    fn unsubscribe(&self) -> Result<()> {
        // nothing to do for this library
        Ok(())
    }
}

#[derive(Clone)]
pub struct AlloyProvider {
    pub provider: RootProvider<PubSubFrontend>,
    pub rt_sync: Arc<RuntimeSync>,
}

impl AlloyProvider {
    pub fn new(url: &str) -> Result<Self> {
        let ws = WsConnect::new(url);
        let rt_sync = Arc::new(RuntimeSync::new()?);
        let alloy_provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;
        Ok(AlloyProvider {
            provider: alloy_provider,
            rt_sync,
        })
    }
}

impl RskProvider for AlloyProvider {
    fn subscribe_blocks(&self) -> Result<impl RskBlockSubscription> {
        AlloyBlockSubscription::new(self.clone())
    }

    fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock> {
        let rpc_call = self
            .provider
            .client()
            .request("eth_getBlockByHash", vec![json!(hash), json!(false)]);

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

    fn disconnect(&self) -> Result<()> {
        // nothing to do for this rsk_provider
        Ok(())
    }
}
