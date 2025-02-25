use crate::event_processor::{event_processor_abi, event_processor_typed};
use crate::sub::AlloySubscription;
use alloy_primitives::B256;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_rpc_types::{Filter, FilterSet, Header, Log};
use anyhow::{anyhow, Context, Result};
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
pub struct AlloyProvider<T = RootProvider>
where
    T: Provider,
{
    inner: T,
    rt_sync: RuntimeSync,
    shutdown_flag: ShutdownFlag,
}

impl AlloyProvider {
    pub fn new(url: &str, shutdown_flag: ShutdownFlag) -> Result<Self> {
        let ws = WsConnect::new(url);
        let rt_sync = RuntimeSync::new().context("On AlloyProvider")?;
        let root_provider = rt_sync
            .run(ProviderBuilder::default().on_ws(ws))
            .context("Failed to create AlloyProvider")?;

        Ok(AlloyProvider {
            inner: root_provider,
            rt_sync,
            shutdown_flag,
        })
    }

    pub(super) fn unsubscribe(&self, name: &str, subscription_id: B256) -> Result<()> {
        self.inner
            .unsubscribe(subscription_id)
            .context(format!("Failed to unsubscribe {subscription_id} @ {name}"))
    }

    fn monitor_shutdown(&self, subscription_id: B256, name: String) {
        let provider_clone = self.inner.clone();
        let unsubscribe_fn = move || {
            debug!("Unsubscribing from {name} for {subscription_id} on shutdown!",);
            provider_clone.unsubscribe(subscription_id).unwrap();
        };

        self.shutdown_flag
            .clone()
            .spawn_shutdown_handler(unsubscribe_fn);
    }

    fn parse_block_provider_response(response: Value) -> Result<Option<RskBlock>> {
        if response.is_null() || !response.is_object() {
            return Ok(None);
        }
        let rpc_block: RskRpcBlock =
            serde_json::from_value(response).context("Deserializing block")?;

        let rsk_block: RskBlock = RskBlock::from(rpc_block);
        Ok(Some(rsk_block))
    }
}

impl RskProvider for AlloyProvider {
    type BlockSubscription = AlloySubscription<Header>;
    type LogSubscription = AlloySubscription<Log>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSubscription> {
        let subscription_request = self.inner.subscribe_blocks();
        let subscription = self
            .rt_sync
            .run(subscription_request)
            .context("Failed to subscribe to blocks")?;
        self.monitor_shutdown(*subscription.local_id(), "Blocks".to_string());
        Ok(AlloySubscription::<Header>::new(subscription, self.clone()))
    }

    fn subscribe_logs(&self, filter: RskSubscriptionFilter) -> Result<Self::LogSubscription> {
        let addresses = AlloySubscription::<Log>::build_addresses(&filter)
            .context("Failed to parse filter addresses")?;

        let filter = Filter {
            block_option: AlloySubscription::<Log>::build_block_option(&filter),
            address: FilterSet::from_iter(addresses),
            topics: AlloySubscription::<Log>::build_topics(&filter)?,
        };

        let subscription_request = self.inner.subscribe_logs(&filter);
        let subscription = self
            .rt_sync
            .run(subscription_request)
            .context("Failed to subscribe to logs")?;

        self.monitor_shutdown(*subscription.local_id(), "Logs".to_string());
        Ok(AlloySubscription::<Log>::new(subscription, self.clone()))
    }

    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>> {
        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByHash", vec![json!(hash), json!(false)]);

        let response: Value = self
            .rt_sync
            .run(rpc_call)
            .context(format!("Getting block {hash} from provider"))?;

        Self::parse_block_provider_response(response)
    }

    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>> {
        let num_hex = format!("0x{:x}", num);

        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByNumber", vec![json!(num_hex), json!(false)]);

        let response: Value = self
            .rt_sync
            .run(rpc_call)
            .context(format!("Getting block {num} from provider"))?;

        Self::parse_block_provider_response(response)
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByNumber", vec![json!("latest"), json!(false)]);

        let response: Value = self
            .rt_sync
            .run(rpc_call)
            .context("Getting best block from provider")?;

        Self::parse_block_provider_response(response)
            .context("Getting best block from provider")?
            .context("None best block")
    }

    fn decode_log(
        &self,
        new_log: RskLog,
        contract_info: &ContractInfo,
    ) -> Result<Option<RskEvent>> {
        if contract_info.abi.is_some() {
            debug!(
                "Dynamic event processing for contract {}",
                contract_info.address
            );
            event_processor_abi::process(
                &contract_info.address,
                new_log,
                contract_info.abi.as_deref().unwrap(),
            )
        } else {
            debug!(
                "Static event processing for contract {}",
                contract_info.address
            );
            rsk_event_result = event_processor_typed::process(new_log);
        }

        rsk_event_result.context("Decoding log")
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
        let rt = Runtime::new().context("Failed to create Tokio runtime")?;
        Ok(RuntimeSync { rt: Arc::new(rt) })
    }

    pub(super) fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType>
    where
        Fut: Future<Output = Result<RetType, Err>>,
        Err: std::error::Error + Send + 'static,
    {
        self.rt.block_on(async {
            future
                .await
                .map_err(|e| anyhow!("Error on RuntimeSync: {:?}", e))
                .context("Async operation failed")
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rpc::AlloyProvider;
    use serde_json::json;
    use serde_json::Value;
    use std::fs;

    const RESPONSE_FILE_PATH: &str = "tests/resources/response.json";

    #[test]
    fn test_parse_provider_response_when_given_valid_data_should_parse_block_succesfully() {
        let data = fs::read_to_string(RESPONSE_FILE_PATH).expect("JSON data should be present");
        let response: Value = serde_json::from_str(&data).expect("Failed to parse JSON");
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result)
            .expect("JSON data should be valid")
            .expect("JSON data should map to RSK block");

        assert_eq!(6086082, block.number());
        assert_eq!(
            "0x2dbb8027f72a9fc147f165646e67db08c130ca698ff2d9fd02058c455b1a1c76",
            block.hash()
        );
        assert_eq!(
            "0x9e1898cf54b4fc263c0025b108f824fa703ed51fb74bdcae6da6e1b8cf728afb",
            block.parent()
        );
    }

    #[test]
    fn test_parse_provider_response_when_given_null_json_should_return_none() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": null
        });
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result)
            .expect("JSON data should be valid");

        assert!(block.is_none());
    }

    #[test]
    fn test_parse_provider_response_when_given_invalid_type_in_data_should_return_error() {
        let data = fs::read_to_string(RESPONSE_FILE_PATH).expect("JSON data should be present");
        let mut response: Value = serde_json::from_str(&data).expect("Failed to parse JSON");
        response["result"]["hash"] = json!(2);
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result);

        assert!(block.is_err());
    }

    #[test]
    fn test_parse_provider_response_when_given_missing_data_should_return_error() {
        let response = json!({
         "jsonrpc": "2.0",
         "id": 1,
         "result": {
           "parent": "0x9e1898cf54b4fc263c0025b108f824fa703ed51fb74bdcae6da6e1b8cf728afb"
         }
        });
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result);

        assert!(block.is_err());
    }

    #[test]
    fn test_parse_provider_response_when_given_null_json_should_not_parse_block_succesfully() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": null
        });
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result)
            .expect("JSON data should be valid");

        assert!(block.is_none());
    }
}
