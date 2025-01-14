use crate::types::{RskBlock, RskLog, RskRpcBlock};
use crate::utils::RuntimeSync;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::{Header, Log};
use anyhow::{anyhow, bail, Result};
use log::trace;
use serde_json::{json, Value};
use std::sync::Arc;

pub trait RskProviderApi {
    fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock>;
    fn get_block_by_number(&self, num: u64) -> Result<RskBlock>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn disconnect(self) -> Result<()>;
}

#[derive(Clone)]
pub struct RskProvider<P>
where
    P: RskProviderApi,
{
    inner: P,
}

impl RskProvider<AlloyProvider> {
    pub fn new(url: &str) -> Self {
        let provider = AlloyProvider::new(url)
            .unwrap_or_else(|e| panic!("Failed to create provider: {:?}", e));
        Self { inner: provider }
    }

    pub fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock> {
        self.inner.get_block_by_hash(hash)
    }

    pub fn get_block_by_number(&self, num: u64) -> Result<RskBlock> {
        self.inner.get_block_by_number(num)
    }

    pub fn get_best_block(&self) -> Result<RskBlock> {
        self.inner.get_best_block()
    }

    pub fn disconnect(self) -> Result<()> {
        self.inner.disconnect()?;
        Ok(())
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

impl RskProviderApi for AlloyProvider {
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

    fn disconnect(self) -> Result<()> {
        drop(self.provider);
        Ok(())
    }
}

pub trait RskSubscriptionApi<T> {
    fn try_next(&mut self) -> Result<Option<T>>;
    fn unsubscribe(self) -> Result<()>;
}

pub struct RskBlockSubscription<P>
where
    P: RskSubscriptionApi<RskBlock>,
{
    inner: P,
}

pub struct RskLogSubscription<P>
where
    P: RskSubscriptionApi<RskLog>,
{
    inner: P,
}

impl RskBlockSubscription<AlloySubscription<Header>> {
    pub fn new(provider: RskProvider<AlloyProvider>) -> Self {
        let subscription: AlloySubscription<Header> =
            <AlloySubscription<Header>>::new(provider.inner)
                .unwrap_or_else(|e| panic!("Failed to create block subscription: {:?}", e));
        Self {
            inner: subscription,
        }
    }

    pub fn next(&mut self) -> Result<Option<RskBlock>> {
        self.inner.try_next()
    }

    pub fn unsubscribe(self) -> Result<()> {
        self.inner.unsubscribe()?;
        Ok(())
    }
}

impl RskLogSubscription<AlloySubscription<Log>> {
    pub fn new(provider: RskProvider<AlloyProvider>) -> Self {
        let subscription: AlloySubscription<Log> = <AlloySubscription<Log>>::new(provider.inner)
            .unwrap_or_else(|e| panic!("Failed to create block subscription: {:?}", e));
        Self {
            inner: subscription,
        }
    }

    fn next(&mut self) -> Result<Option<RskLog>> {
        self.inner.try_next()
    }

    fn unsubscribe(self) -> Result<()> {
        self.inner.unsubscribe()?;
        Ok(())
    }
}

pub struct AlloySubscription<T> {
    sub: Subscription<T>,
    provider: AlloyProvider,
}

impl AlloySubscription<Header> {
    fn new(provider: AlloyProvider) -> Result<Self> {
        let subscription_request = provider.provider.subscribe_blocks();
        let sub = provider.rt_sync.run(subscription_request)?;
        Ok(Self { sub, provider })
    }
}

impl AlloySubscription<Log> {
    fn new(provider: AlloyProvider) -> Result<Self> {
        todo!("Implement me!")
    }
}

impl RskSubscriptionApi<RskBlock> for AlloySubscription<Header> {
    fn try_next(&mut self) -> Result<Option<RskBlock>> {
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

        // TODO(iago) resilience when block is not found
        Ok(Some(self.provider.get_block_by_hash(&new_block_hash)?))
    }

    fn unsubscribe(self) -> Result<()> {
        // TODO(iago) nothing to do apparently for this provider? confirm
        Ok(())
    }
}

impl RskSubscriptionApi<RskLog> for AlloySubscription<Log> {
    fn try_next(&mut self) -> Result<Option<RskLog>> {
        todo!("Implement me!")
    }

    fn unsubscribe(self) -> Result<()> {
        // TODO(iago) nothing to do apparently for this provider? confirm
        Ok(())
    }
}
