use crate::event_processor::{event_processor_abi, event_processor_typed};
use crate::sub::AlloySubscription;
use alloy_primitives::B256;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::PubSubFrontend;
use alloy_rpc_types::{Filter, FilterSet, Header, Log};
use anyhow::{anyhow, Result};
use common::rsk_provider::{RskProvider, RskSubscriptionFilter};
use common::shutdown_flag::ShutdownFlag;
use common::types::{ContractInfo, RskBlock, RskEvent, RskLog, RskRpcBlock};
use log::debug;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::Runtime;
// TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15. Review these methods accordingly.
// TODO(Jira) error resilience: https://rsklabs.atlassian.net/browse/UB-28

#[derive(Clone)]
pub struct AlloyProvider {
    inner: RootProvider<PubSubFrontend>,
    rt_sync: RuntimeSync,
    shutdown_flag: ShutdownFlag,
}

impl AlloyProvider {
    pub fn new(url: &str, shutdown_flag: ShutdownFlag) -> Result<Self> {
        let ws = WsConnect::new(url);
        let rt_sync = RuntimeSync::new()?;
        let root_provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;

        Ok(AlloyProvider {
            inner: root_provider,
            rt_sync,
            shutdown_flag,
        })
    }

    pub(super) fn unsubscribe(&self, subscription_id: B256) -> Result<()> {
        self.inner.unsubscribe(subscription_id)?;
        Ok(())
    }

    fn monitor_shutdown(&self, subscription_id: B256, name: String) {
        let provider_clone = self.inner.clone();
        let unsubscribe_fn = move || {
            debug!("Unsubscribing from {} on shutdown!", name);
            provider_clone.unsubscribe(subscription_id).unwrap();
        };

        self.shutdown_flag
            .clone()
            .spawn_shutdown_handler(unsubscribe_fn);
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
    type BlockSubscription = AlloySubscription<Header>;
    type LogSubscription = AlloySubscription<Log>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSubscription> {
        let subscription_request = self.inner.subscribe_blocks();
        let subscription = self.rt_sync.run(subscription_request)?;
        self.monitor_shutdown(*subscription.local_id(), "Blocks".to_string());
        Ok(AlloySubscription::<Header>::new(subscription, self.clone()))
    }

    fn subscribe_logs(&self, filter: RskSubscriptionFilter) -> Result<Self::LogSubscription> {
        let filter = Filter {
            block_option: AlloySubscription::<Log>::build_block_option(&filter),
            address: FilterSet::from_iter(AlloySubscription::<Log>::build_addresses(&filter)?),
            topics: AlloySubscription::<Log>::build_topics(&filter)?,
        };

        let subscription_request = self.inner.subscribe_logs(&filter);
        let subscription = self.rt_sync.run(subscription_request)?;
        self.monitor_shutdown(*subscription.local_id(), "Logs".to_string());
        Ok(AlloySubscription::<Log>::new(subscription, self.clone()))
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

    fn decode_log(
        &self,
        new_log: RskLog,
        contract_info: &ContractInfo,
    ) -> Result<Option<RskEvent>> {
        if contract_info.abi_file.is_some() {
            debug!(
                "ABI based event processing for contract {}",
                contract_info.address
            );
            event_processor_abi::process(
                &contract_info.address,
                new_log,
                contract_info.abi_file.as_deref().unwrap(),
            )
        } else {
            debug!(
                "Static event processing for contract {}",
                contract_info.address
            );
            event_processor_typed::process(new_log)
        }
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
    pub(super) fn new() -> Result<Self> {
        // Note: we cannot use Builder::new_current_thread() because Alloy needs multiple to work
        let rt = Runtime::new().expect("Failed to create Tokio runtime (unrecoverable)");
        Ok(RuntimeSync { rt: Arc::new(rt) })
    }

    pub(super) fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType>
    where
        Fut: Future<Output=Result<RetType, Err>>,
        Err: std::error::Error + Send + 'static,
    {
        self.rt.block_on(async {
            future
                .await
                .map_err(|e| anyhow!("Error on RuntimeSync: {:?}", e))
        })
    }
}
