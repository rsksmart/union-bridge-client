use crate::{
    alloy_rsk_provider::{
        event_processor::{event_processor_abi, event_processor_typed},
        sub::AlloySubscription,
    },
    rsk_provider::{RskProvider, RskSubscriptionFilter},
    shutdown_flag::ShutdownFlag,
    types::{
        Address, BlockHash, BlockNumber, ContractInfo, RskBlock, RskEvent, RskLog, RskRpcBlock,
        RskRpcLog, ToHexString,
    },
};

use crate::runtime_sync::RuntimeSync;
use alloy_primitives::B256;
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types::{Filter, FilterSet, Header, Log};
use alloy_transport::layers::RetryBackoffLayer;
use anyhow::{Context, Result, bail};
use log::debug;
use serde_json::{Value, json};
use std::future::Future;

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
        // Set up the sync-runtime
        let rt_sync = RuntimeSync::new().context("On AlloyProvider")?;

        // Prepare the WS transport
        let ws = WsConnect::new(url);

        // Build your retry layer
        let max_retry = 6; // wait time for retry is 2^attempt, so: 1s + 2s + 4s + 8s = 15s max <=> half a block time
        let initial_backoff_ms = 500;
        let cups = 100;
        let retry_layer = RetryBackoffLayer::new(max_retry, initial_backoff_ms, cups);

        // Block on the RpcClient‐builder future
        let client: RpcClient = rt_sync
            .run(RpcClient::builder().layer(retry_layer).ws(ws))
            .context("Failed to build RpcClient with retry layer")?;

        // Synchronously feed that client into ProviderBuilder
        let root_provider = ProviderBuilder::default().connect_client(client);

        Ok(AlloyProvider {
            inner: root_provider,
            rt_sync,
            shutdown_flag,
        })
    }

    pub fn unsubscribe(&self, name: &str, subscription_id: B256) -> Result<()> {
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

    fn parse_logs_provider_response(response: Value) -> Result<Vec<RskLog>> {
        if response.is_null() || !response.is_array() {
            bail!("Expected array in logs response, got: {:?}", response);
        }

        let rpc_logs: Vec<RskRpcLog> =
            serde_json::from_value(response).context("Deserializing logs array")?;

        let rsk_logs: Vec<RskLog> = rpc_logs.into_iter().map(RskLog::from).collect();

        Ok(rsk_logs)
    }

    fn run<Fut, Err>(&self, rpc_call: Fut) -> Result<Value>
    where
        Fut: Future<Output = Result<Value, Err>> + Clone + Send + 'static,
        Err: std::error::Error + Send + Sync + 'static,
    {
        let val = self.rt_sync.run(rpc_call.clone()).context("RPC failed")?;

        Ok(val)
    }
}

impl RskProvider for AlloyProvider {
    type BlockSubscription = AlloySubscription<Header>;
    type LogSubscription = AlloySubscription<Log>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSubscription> {
        let subscription_request = self.inner.subscribe_blocks();
        let subscription = self
            .rt_sync
            .run(subscription_request.into_future())
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
            .run(subscription_request.into_future())
            .context("Failed to subscribe to logs")?;

        self.monitor_shutdown(*subscription.local_id(), "Logs".to_string());
        Ok(AlloySubscription::<Log>::new(subscription, self.clone()))
    }

    fn get_block_by_hash(&self, hash: BlockHash) -> Result<Option<RskBlock>> {
        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByHash", vec![json!(hash), json!(false)]);

        self.run(rpc_call)
            .context(format!("Getting block {hash} from provider"))
            .and_then(|response| Self::parse_block_provider_response(response))
    }

    fn get_block_by_number(&self, num: BlockNumber) -> Result<Option<RskBlock>> {
        let num_hex = format!("{:#x}", num.value());

        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByNumber", vec![json!(num_hex), json!(false)]);

        self.run(rpc_call)
            .context(format!("Getting block {num} from provider"))
            .and_then(|response| Self::parse_block_provider_response(response))
    }

    fn get_best_block(&self) -> Result<RskBlock> {
        let rpc_call = self
            .inner
            .client()
            .request("eth_getBlockByNumber", vec![json!("latest"), json!(false)]);

        self.run(rpc_call)
            .context("Getting block latest from provider")
            .and_then(|response| Self::parse_block_provider_response(response))
            .context("Getting best block from provider")?
            .context("None best block")
    }

    fn get_uncle_by_hash_and_index(&self, hash: BlockHash, index: u64) -> Result<Option<RskBlock>> {
        // Convert index to hexadecimal format
        let hex_index = format!("{:#x}", index);

        let rpc_call = self.inner.client().request(
            "eth_getUncleByBlockHashAndIndex",
            vec![json!(hash), json!(hex_index)],
        );

        self.run(rpc_call)
            .context(format!("Getting block {hash} from provider"))
            .and_then(|response| Self::parse_block_provider_response(response))
    }

    fn get_logs(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        addrs: &Vec<Address>,
    ) -> Result<Vec<RskLog>> {
        let addrs: Vec<String> = addrs.iter().map(|addr| addr.to_hex_string()).collect();

        let params = json!([{
            "fromBlock": from.to_hex_string(),
            "toBlock": to.to_hex_string(),
            "address": addrs,
        }]);

        let rpc_call = self.inner.client().request("eth_getLogs", params);

        self.run(rpc_call)
            .context(format!(
                "Getting logs for addresses [{}] from block {} to {}",
                addrs.join(", "),
                from,
                to
            ))
            .and_then(|response| Self::parse_logs_provider_response(response))
    }

    fn decode_log(
        &self,
        new_log: RskLog,
        contract_info: &ContractInfo,
    ) -> Result<Option<RskEvent>> {
        let rsk_event_result;

        if let Some(abi) = &contract_info.abi {
            debug!(
                "Dynamic event processing for contract {}",
                contract_info.address
            );
            rsk_event_result = event_processor_abi::process(contract_info.address, new_log, &abi);
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

#[cfg(test)]
mod tests {
    use crate::types::{DataBytes, LogTopic, TxHash};
    use crate::{
        alloy_rsk_provider::rpc::AlloyProvider,
        types::{Address, BlockHash, BlockNumber},
    };
    use serde_json::{Value, json};
    use std::fs;

    const BLOCK_RESPONSE_FILE_PATH: &str = "tests/resources/block_response.json";
    const LOG_RESPONSE_FILE_PATH: &str = "tests/resources/log_response.json";

    #[test]
    fn test_parse_provider_block_response_when_given_valid_data_should_parse_block_successfully() {
        let data =
            fs::read_to_string(BLOCK_RESPONSE_FILE_PATH).expect("JSON data should be present");
        let response: Value = serde_json::from_str(&data).expect("Failed to parse JSON");
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result.clone())
            .expect("JSON data should be valid")
            .expect("JSON data should map to RSK block");

        let expected_hash = BlockHash::try_from(
            result["hash"]
                .as_str()
                .expect("Block hash should be a string"),
        )
        .expect("Invalid hex string in JSON");

        let expected_parent = BlockHash::try_from(
            result["parentHash"]
                .as_str()
                .expect("Parent hash should be a string"),
        )
        .expect("Invalid hex string in JSON");

        let expected_uncle_hash = BlockHash::try_from(
            result["uncles"][0]
                .as_str()
                .expect("Uncle hash should be a string"),
        )
        .expect("Invalid hex string in JSON");

        assert_eq!(BlockNumber::from(6161807), block.number());
        assert_eq!(expected_hash, block.hash());
        assert_eq!(expected_parent, block.parent_hash());
        assert_eq!(1, block.uncles().len());
        assert_eq!(expected_uncle_hash, block.uncles()[0]);
    }

    #[test]
    fn test_parse_provider_block_response_when_given_null_json_should_return_none() {
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
    fn test_parse_provider_block_response_when_given_invalid_type_in_data_should_return_error() {
        let data =
            fs::read_to_string(BLOCK_RESPONSE_FILE_PATH).expect("JSON data should be present");
        let mut response: Value = serde_json::from_str(&data).expect("Failed to parse JSON");
        response["result"]["hash"] = json!(2);
        let result: Value = response["result"].clone();

        let block = AlloyProvider::parse_block_provider_response(result);

        assert!(block.is_err());
    }

    #[test]
    fn test_parse_provider_block_response_when_given_missing_data_should_return_error() {
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
    fn test_parse_provider_block_response_when_given_null_json_should_not_parse_block_succesfully()
    {
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
    fn test_parse_provider_logs_response_when_given_valid_data_should_parse_logs_successfully() {
        let data = fs::read_to_string(LOG_RESPONSE_FILE_PATH).expect("JSON data should be present");
        let response: Value = serde_json::from_str(&data).expect("Failed to parse JSON");
        let result: Value = response["result"].clone();

        let logs = AlloyProvider::parse_logs_provider_response(result.clone())
            .expect("JSON data should be valid");

        let expected_address = Address::try_from(
            result[0]["address"]
                .as_str()
                .expect("Log address should be a string"),
        )
        .expect("Invalid hex string in JSON");

        let expected_block_hash = BlockHash::try_from(
            result[0]["blockHash"]
                .as_str()
                .expect("Block hash should be a string"),
        )
        .expect("Invalid hex string in JSON");

        let expected_block_number = BlockNumber::try_from(
            result[0]["blockNumber"]
                .as_str()
                .expect("Block number should be a string"),
        )
        .expect("Invalid hex string in JSON");

        let expected_tx_hash = TxHash::try_from(
            result[0]["transactionHash"]
                .as_str()
                .expect("Transaction hash should be a string"),
        )
        .expect("Invalid hex string in JSON");

        let expected_data = &DataBytes::from_hex_str(
            result[0]["data"]
                .as_str()
                .expect("Log data should be a string"),
        )
        .expect("Failed to parse expected data");

        let expected_topics: Vec<LogTopic> = result[0]["topics"]
            .as_array()
            .expect("Topics should be an array")
            .iter()
            .map(|t| {
                LogTopic::try_from(t.as_str().expect("Topic should be a string"))
                    .expect("Invalid hex string in JSON")
            })
            .collect();

        assert_eq!(expected_address, logs[0].info().address());
        assert_eq!(expected_block_hash, logs[0].info().block_hash());
        assert_eq!(expected_block_number, logs[0].info().block_number());
        assert_eq!(expected_tx_hash, logs[0].info().tx_hash());
        assert_eq!(expected_data, logs[0].event().data());
        assert_eq!(&expected_topics, logs[0].event().topics());
    }

    #[test]
    fn test_parse_provider_log_response_when_given_invalid_type_in_data_should_return_error() {
        let data = fs::read_to_string(LOG_RESPONSE_FILE_PATH).expect("JSON data should be present");

        let mut response: Value = serde_json::from_str(&data).expect("Failed to parse JSON");

        response["result"][0]["data"] = json!(2);

        let result: Value = response["result"].clone();
        let log = AlloyProvider::parse_logs_provider_response(result);

        assert!(log.is_err());
        let err_msg = log.unwrap_err().to_string();
        assert_eq!(err_msg, "Deserializing logs array");
    }

    #[test]
    fn test_parse_provider_log_response_when_given_missing_data_should_return_error() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": [
                {
                    "address": "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761",
                    "data": "0xabcdef"
                }
            ]
        });

        let result: Value = response["result"].clone();
        let log = AlloyProvider::parse_logs_provider_response(result);

        assert!(log.is_err());
        let err_msg = log.unwrap_err().to_string();
        assert_eq!(err_msg, "Deserializing logs array");
    }

    #[test]
    fn test_parse_provider_log_response_when_given_null_json_should_not_parse_block_succesfully() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": null
        });
        let result: Value = response["result"].clone();

        let log = AlloyProvider::parse_logs_provider_response(result);

        assert!(log.is_err());
        let err_msg = log.unwrap_err().to_string();
        assert_eq!(err_msg, "Expected array in logs response, got: Null");
    }
}
