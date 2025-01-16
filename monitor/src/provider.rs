use crate::types::{RskBlock, RskLog, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::{Header, Log};
use anyhow::{anyhow, bail, Result};
use log::trace;
use serde_json::{json, Value};
use std::sync::Arc;

pub trait RskProvider {
    fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock>;
    fn get_block_by_number(&self, num: u64) -> Result<RskBlock>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn disconnect(&self) -> Result<()>;
}

pub struct RskApi<P>
where
    P: RskProvider,
{
    provider: P,
}

impl RskApi<AlloyProvider> {
    pub fn new(url: &str) -> Self {
        let provider = AlloyProvider::new(url)
            .unwrap_or_else(|e| panic!("Failed to create provider: {:?}", e));
        Self { provider }
    }
}

impl<P> RskProvider for RskApi<P>
where
    P: RskProvider,
{
    fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock> {
        self.provider.get_block_by_hash(hash)
    }

    fn get_block_by_number(&self, num: u64) -> Result<RskBlock> {
        self.provider.get_block_by_number(num)
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        self.provider.get_best_block()
    }

    fn disconnect(&self) -> Result<()> {
        self.provider.disconnect()
    }
}

#[derive(Clone)]
pub struct AlloyProvider {
    provider: RootProvider<PubSubFrontend>,
    rt_sync: Arc<RuntimeSync>,
}

impl AlloyProvider {
    fn new(url: &str) -> Result<Self> {
        let ws = WsConnect::new(url);
        let rt_sync = Arc::new(RuntimeSync::new()?);
        let provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;
        Ok(AlloyProvider { provider, rt_sync })
    }
}

impl RskProvider for AlloyProvider {
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
        // nothing to do for this provider
        Ok(())
    }
}

pub trait RskBlockSubscription {
    fn try_next(&mut self) -> Result<Option<RskBlock>>;
    fn unsubscribe(self) -> Result<()>;
}

pub struct RskBlockSubscriptionApi<S>
where
    S: RskBlockSubscription,
{
    subscription: S,
}

impl RskBlockSubscriptionApi<AlloyBlockSubscription> {
    pub fn new(provider: &RskApi<AlloyProvider>) -> Self {
        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
        let subscription = AlloyBlockSubscription::new(&provider.provider.clone())
            .unwrap_or_else(|e| panic!("Failed to create block subscription: {:?}", e));
        Self { subscription }
    }
}

impl<S> RskBlockSubscription for RskBlockSubscriptionApi<S>
where
    S: RskBlockSubscription,
{
    fn try_next(&mut self) -> Result<Option<RskBlock>> {
        self.subscription.try_next()
    }

    fn unsubscribe(self) -> Result<()> {
        self.subscription.unsubscribe()
    }
}

pub struct AlloyBlockSubscription {
    subscription: Subscription<Header>,
    provider: AlloyProvider,
}

impl AlloyBlockSubscription {
    fn new(provider: &AlloyProvider) -> Result<Self> {
        let subscription_request = provider.provider.subscribe_blocks();
        let sub = provider.rt_sync.run(subscription_request)?;
        Ok(Self {
            subscription: sub,
            provider: provider.clone(),
        })
    }
}

impl RskBlockSubscription for AlloyBlockSubscription {
    fn try_next(&mut self) -> Result<Option<RskBlock>> {
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

    fn unsubscribe(self) -> Result<()> {
        Ok(drop(self.subscription))
    }
}

pub trait RskLogSubscription {
    fn try_next(&mut self) -> Result<Option<RskLog>>;
    fn unsubscribe(&self) -> Result<()>;
}

pub struct RskLogSubscriptionApi<S>
where
    S: RskLogSubscription,
{
    subscription: S,
}

#[allow(dead_code)]
impl RskLogSubscriptionApi<AlloyLogSubscription> {
    pub fn new(_provider: &RskApi<AlloyProvider>) -> Self {
        todo!("Implement RskLogSubscriptionApi::new")
    }
}

impl<S> RskLogSubscription for RskLogSubscriptionApi<S>
where
    S: RskLogSubscription,
{
    fn try_next(&mut self) -> Result<Option<RskLog>> {
        self.subscription.try_next()
    }

    fn unsubscribe(&self) -> Result<()> {
        Ok(self.subscription.unsubscribe()?)
    }
}

#[allow(dead_code)]
pub struct AlloyLogSubscription {
    subscription: Subscription<Log>,
    provider: AlloyProvider,
}

#[allow(dead_code)]
impl AlloyLogSubscription {
    fn new(_provider: &AlloyProvider) -> Result<Self> {
        todo!("Implement AlloyLogSubscription::new")
    }
}

impl RskLogSubscription for AlloyLogSubscription {
    fn try_next(&mut self) -> Result<Option<RskLog>> {
        todo!("Implement AlloyLogSubscription::try_next")
    }

    fn unsubscribe(&self) -> Result<()> {
        todo!("Implement AlloyLogSubscription::unsubscribe")
    }
}
