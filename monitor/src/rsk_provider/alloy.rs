use crate::rsk_provider::provider::{RskProvider, RskSubscription};
use crate::types::{RskBlock, RskLog, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::Header;
use anyhow::{anyhow, bail, Result};
use log::debug;
use serde_json::{json, Value};
use std::sync::Arc;

// TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15. Review these methods accordingly.
// TODO(Jira) error resilience: https://rsklabs.atlassian.net/browse/UB-28

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

impl RskSubscription<RskBlock> for AlloyBlockSubscription {
    fn next(&mut self) -> Result<RskBlock> {
        // TODO(iago) try to close the subscription on shutdown to avoid waiting on a next block
        let header = self.subscription.blocking_recv_any()?;

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

        // TODO(Jira) tmp approach, try to get the required block data from the subscription itself (check Rsk and Alloy impl): https://rsklabs.atlassian.net/browse/UB-36
        let new_block = self.provider.get_block_by_hash(&new_block_hash)?;
        if new_block.is_none() {
            bail!(
                "hash informed by Provider {} does not exist",
                new_block_hash
            );
        }

        Ok(new_block.unwrap())
    }

    fn unsubscribe(&self) -> Result<()> {
        self.provider
            .provider
            .unsubscribe(*self.subscription.local_id())?;
        Ok(())
    }
}

struct AlloyLogSubscription {}

impl AlloyLogSubscription {
    fn new() -> Result<Self> {
        todo!("Implement AlloyLogSubscription::new")
    }
}

impl RskSubscription<RskLog> for AlloyLogSubscription {
    fn next(&mut self) -> Result<RskLog> {
        todo!("Implement AlloyLogSubscription::next")
    }

    fn unsubscribe(&self) -> Result<()> {
        todo!("Implement AlloyLogSubscription::unsubscribe")
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

    fn parse_provider_response(response: Value) -> Result<Option<RskBlock>> {
        if response.is_null() || !response.is_object() {
            return Ok(None);
        }
        let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
        let rsk_block: RskBlock = RskBlock::from(rpc_block);
        Ok(Some(rsk_block))
    }
}

impl RskProvider for AlloyProvider {
    fn subscribe_blocks(&self) -> Result<impl RskSubscription<RskBlock>> {
        AlloyBlockSubscription::new(self.clone())
    }

    fn subscribe_logs(&self) -> Result<impl RskSubscription<RskLog>> {
        AlloyLogSubscription::new()
    }

    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>> {
        let rpc_call = self
            .provider
            .client()
            .request("eth_getBlockByHash", vec![json!(hash), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;
        Self::parse_provider_response(response)
    }

    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>> {
        let num_hex = format!("0x{:x}", num);

        let rpc_call = self
            .provider
            .client()
            .request("eth_getBlockByNumber", vec![json!(num_hex), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;
        Self::parse_provider_response(response)
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        let rpc_call = self
            .provider
            .client()
            .request("eth_getBlockByNumber", vec![json!("latest"), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;
        Self::parse_provider_response(response)?.ok_or(anyhow!("Could not get best block"))
    }

    fn disconnect(&self) -> Result<()> {
        // nothing to do for this rsk_provider
        Ok(())
    }
}
