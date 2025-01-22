use crate::rsk_provider::provider::{RskProvider, RskSubscription};
use crate::types::{RskBlock, RskLog, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::Header;
use anyhow::{anyhow, bail, Result};
use log::{debug, trace};
use serde_json::{json, Value};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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
    fn next(&mut self) -> Result<Option<RskBlock>> {
        let header = match self.subscription.try_recv_any() {
            Ok(header) => header,
            Err(_) => {
                trace!("No new block yet");
                return Ok(None);
            }
        };

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

        let new_block = self
            .provider
            .get_block_by_hash(&new_block_hash)
            .expect("hash informed by Provider does not exist (unrecoverable)");
        Ok(new_block)
    }

    fn unsubscribe(&self) -> Result<()> {
        // nothing to do for this library
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
    fn next(&mut self) -> Result<Option<RskLog>> {
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
        let mut best_block = None;

        // covers the edge case of a reorg to a lower block num: eth_blockNumber result does not exist for eth_getBlockByNumber
        for _ in 0..10 {
            let rpc_call = self.provider.client().request_noparams("eth_blockNumber");
            let response: Value = self.rt_sync.run(rpc_call)?;
            let number_hex: String = serde_json::from_value(response)?;
            let number_dec = u64::from_str_radix(&number_hex.trim_start_matches("0x"), 16)?;
            best_block = self.get_block_by_number(number_dec)?;

            if best_block.is_some() {
                break;
            }

            thread::sleep(Duration::from_secs(1));
        }

        Ok(best_block.expect("Failed to get best block after 10 attempts (unrecoverable)"))
    }

    fn disconnect(&self) -> Result<()> {
        // nothing to do for this rsk_provider
        Ok(())
    }
}
