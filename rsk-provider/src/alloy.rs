use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::{Header, Log};
use anyhow::{anyhow, Result};
use common::rsk_provider::{RskProvider, RskProviderError, RskSubscription};
use common::shutdown_flag::ShutdownFlag;
use common::types::{RskBlock, RskLog, RskRpcBlock};
use log::debug;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;
// TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15. Review these methods accordingly.
// TODO(Jira) error resilience: https://rsklabs.atlassian.net/browse/UB-28

struct AlloySubscription<T> {
    subscription: Subscription<T>,
    provider: AlloyProvider,
}

impl<T: serde::de::DeserializeOwned> AlloySubscription<T> {
    fn next(&mut self) -> Result<SubscriptionItem<T>, RskProviderError> {
        match self.subscription.blocking_recv_any() {
            Ok(header) => Ok(header),
            Err(RecvError::Closed) => Err(RskProviderError::Closed),
            Err(e) => Err(RskProviderError::Other(format!("{:?}", e))),
        }
    }
}

impl AlloySubscription<Header> {
    fn new(provider: AlloyProvider, shutdown_flag: ShutdownFlag) -> Result<Self> {
        let subscription_request = provider.inner.subscribe_blocks();
        let subscription = provider.rt_sync.run(subscription_request)?;

        let subscription_id = *subscription.local_id();
        let provider_clone = provider.clone();
        let unsubscribe_fn = move || {
            debug!("Unsubscribing from blocks on shutdown!");
            provider_clone.inner.unsubscribe(subscription_id).unwrap();
        };

        shutdown_flag.spawn_shutdown_handler(unsubscribe_fn);

        Ok(Self {
            subscription,
            provider,
        })
    }
}

impl RskSubscription<RskBlock> for AlloySubscription<Header> {
    fn next(&mut self) -> Result<RskBlock, RskProviderError> {
        // TODO(iago) try to close the subscription on shutdown to avoid waiting on a next block
        let header = self.next()?;

        debug!("Received header: {:?}", header);

        let new_block_header_raw = match header {
            SubscriptionItem::Other(raw_json) => raw_json.get().to_string(),
            _ => {
                return Err(RskProviderError::Other(format!(
                    "Unexpected SubscriptionItem: {:?}",
                    header
                )));
            }
        };

        let new_block_header: Value = serde_json::from_str(&*new_block_header_raw)?;
        let new_block_hash = new_block_header["hash"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing hash field"))?;

        // TODO(Jira) tmp approach, try to get the required block data from the subscription itself (check Rsk and Alloy impl): https://rsklabs.atlassian.net/browse/UB-36
        let new_block = self.provider.get_block_by_hash(&new_block_hash)?;
        if new_block.is_none() {
            return Err(RskProviderError::Other(format!(
                "hash informed by Provider {} does not exist",
                new_block_hash
            )));
        }

        Ok(new_block.unwrap())
    }

    fn unsubscribe(&self) -> Result<()> {
        self.provider
            .inner
            .unsubscribe(*self.subscription.local_id())?;
        Ok(())
    }
}

impl AlloySubscription<Log> {
    fn new(_provider: AlloyProvider, _shutdown_flag: ShutdownFlag) -> Result<Self> {
        unimplemented!()
    }
}

impl RskSubscription<RskLog> for AlloySubscription<Log> {
    fn next(&mut self) -> Result<RskLog, RskProviderError> {
        todo!("Implement AlloyLogSubscription::next")
    }

    fn unsubscribe(&self) -> Result<()> {
        todo!("Implement AlloyLogSubscription::unsubscribe")
    }
}

#[derive(Clone)]
pub struct AlloyProvider {
    inner: RootProvider<PubSubFrontend>,
    rt_sync: RuntimeSync,
}

impl AlloyProvider {
    pub fn new(url: &str) -> Result<Self> {
        let ws = WsConnect::new(url);
        let rt_sync = RuntimeSync::new()?;
        let alloy_provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;
        Ok(AlloyProvider {
            inner: alloy_provider,
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
    fn subscribe_blocks(
        &self,
        shutdown_flag: ShutdownFlag,
    ) -> Result<impl RskSubscription<RskBlock>> {
        AlloySubscription::<Header>::new(self.clone(), shutdown_flag)
    }

    fn subscribe_logs(&self, shutdown_flag: ShutdownFlag) -> Result<impl RskSubscription<RskLog>> {
        AlloySubscription::<Log>::new(self.clone(), shutdown_flag)
    }

    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>> {
        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByHash", vec![json!(hash), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;
        Self::parse_provider_response(response)
    }

    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>> {
        let num_hex = format!("0x{:x}", num);

        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByNumber", vec![json!(num_hex), json!(false)]);

        let response: Value = self.rt_sync.run(rpc_call)?;
        Self::parse_provider_response(response)
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        let rpc_call = self
            .inner
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

// This struct is a wrapper around tokio::runtime::Runtime that allows for synchronous execution of
// async functions.
// Note 1: it is discouraged to start several runtimes, so use with caution.
// Note 2: we need Tokio because Alloy requires a Tokio Reactor to work
#[derive(Clone)]
struct RuntimeSync {
    rt: Arc<Runtime>,
}

impl RuntimeSync {
    pub fn new() -> Result<Self> {
        // Note: we cannot use Builder::new_current_thread() because Alloy needs multiple to work
        let rt = Runtime::new().expect("Failed to create Tokio runtime (unrecoverable)");
        Ok(RuntimeSync { rt: Arc::new(rt) })
    }

    pub fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType>
    where
        Fut: Future<Output = Result<RetType, Err>>,
        Err: std::error::Error + Send + 'static,
    {
        self.rt.block_on(async {
            future
                .await
                .map_err(|e| anyhow!("Error on RuntimeSync: {:?}", e))
        })
    }
}
