use crate::{
    config::REQUIRED_CONFIRMATIONS,
    event_processor::{
        EventProcessor,
        blockchain_tracker::{BlockConfirmations, BlockchainView},
    },
    types::{EventWithBlock, RskPegManagerEvents},
};
use anyhow::{Context, Result, bail};
use bitvmx_client::{program::variables::VariableTypes, types::IncomingBitVMXApiMessages};
use common::{
    msg_broker::{
        broker::{BROKER_SERVER_ID, BrokerClientApi},
        types::{FromServer, ToServer},
    },
    types::RskBlockAndUncles,
};
use log::{error, info};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{cell::RefCell, env, rc::Rc};
use union_contracts::bindings::peg_manager::PegManager::PeginRequested;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct UnconfirmedEvent<T: Clone> {
    event_id: Uuid,
    data: EventWithBlock<T>,
    confirmations: Rc<RefCell<BlockConfirmations>>,
}

impl<T: Clone> UnconfirmedEvent<T> {
    fn new(data: EventWithBlock<T>, required_confirmations: u32) -> Self {
        let event_id = Uuid::new_v4();
        let confirmations = BlockConfirmations::new(
            event_id.to_string(),
            data.block_number,
            required_confirmations,
        );
        let rc_confirmations = Rc::new(RefCell::new(confirmations));

        Self {
            event_id,
            data,
            confirmations: rc_confirmations,
        }
    }

    pub fn confirmations(&self) -> Rc<RefCell<BlockConfirmations>> {
        self.confirmations.clone()
    }

    fn is_confirmed(&self) -> bool {
        self.confirmations.borrow().is_confirmed()
    }
}

pub struct PeginProcessor<T: BrokerClientApi> {
    http_client: Client,
    bitvmx_broker: T,
    blockchain: BlockchainView,
    pegin_requested_events: Vec<UnconfirmedEvent<PeginRequested>>,
}

impl<T: BrokerClientApi> PeginProcessor<T> {
    pub fn new(bitvmx_broker: T) -> Self {
        Self {
            http_client: Client::new(),
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            pegin_requested_events: Vec::new(),
        }
    }

    fn proxy_request(&self, method_name: &str, json_value: &Value) -> Result<Value> {
        let url = format!("{}/{}", Self::get_tx_dispatcher_url(), method_name);

        let res = self
            .http_client
            .post(url)
            .json(json_value)
            .send()
            .context("Failed to send request")?;

        if res.status().is_success() {
            let result: Value = res.json().context("Failed to parse response as JSON")?;
            Ok(result)
        } else {
            let status = res.status();
            let text = res.text().unwrap_or_else(|_| "<no body>".to_string());
            bail!("Request failed: {status} - {text}");
        }
    }

    fn send_response_to_bitvmx(&self, method: &str, payload: Value) -> Result<bool> {
        match method {
            "pegin-address" => {
                let msg = ToServer::TemporaryPegInAddressMockedBitVMX(payload);
                Ok(self.bitvmx_broker.send(BROKER_SERVER_ID, msg)?)
            }
            "register-pegin" => {
                // No response needed for register-pegin
                Ok(true)
            }
            _ => bail!("Unsupported method name for BitVMX response: {}", method),
        }
    }

    fn process_confirmed_pegin_requested_events(&mut self) -> Result<()> {
        let confirmed_events: Vec<_> = self
            .pegin_requested_events
            .iter()
            .filter(|event| event.is_confirmed())
            .map(|event| (event.event_id, event.data.inner.clone()))
            .collect();

        let mut processed_events = Vec::new();
        confirmed_events.iter().for_each(|(event_id, data)| {
            match self.handle_confirmed_event(*event_id, "PeginRequested", data) {
                Ok(_) => {
                    info!(
                        "Successfully processed confirmed PeginRequested event: {}",
                        event_id
                    );
                    processed_events.push(*event_id);
                }
                Err(e) => {
                    // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
                    error!(
                        "Error processing confirmed PeginRequested event {}: {}",
                        event_id, e
                    );
                }
            }
        });

        // Only remove successfully processed events - keep unconfirmed and failed events
        self.pegin_requested_events
            .retain(|event| !event.is_confirmed() || !processed_events.contains(&event.event_id));

        Ok(())
    }

    fn handle_confirmed_event<E: serde::Serialize>(
        &mut self,
        event_id: Uuid,
        variable_name: &str,
        data: E,
    ) -> Result<()> {
        let data = json!(data).to_string();

        self.bitvmx_broker.send(
            BROKER_SERVER_ID,
            ToServer::ToBitVMX(IncomingBitVMXApiMessages::SetVar(
                event_id,
                variable_name.to_string(),
                VariableTypes::String(data),
            )),
        )?;

        self.blockchain
            .remove_observer(event_id.to_string().as_str());

        Ok(())
    }

    fn get_tx_dispatcher_url() -> String {
        // Env var because the http server is temporary: defined for docker, defaulting otherwise
        env::var("TRANSACTION_DISPATCHER_URL").unwrap_or("http://0.0.0.0:3000".to_string())
    }
}

impl<T: BrokerClientApi> EventProcessor for PeginProcessor<T> {
    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> Result<()> {
        match event {
            FromServer::FromBitVMX(method, data)
                if matches!(method.as_str(), "pegin-address" | "register-pegin") =>
            {
                info!(
                    "Handling BitVMX event. Method: {}, Payload: {:?}",
                    method, data
                );

                let result = self.proxy_request(method, data)?;
                info!(
                    "Successfully proxied request for method '{}'. Response: {}",
                    method, result
                );

                self.send_response_to_bitvmx(method, result.clone())?;
                info!(
                    "Successfully sent response to BitVMX broker for method '{}'. Payload: {}",
                    method, result
                );
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::PeginRequested(data) => {
                info!("Handling PeginRequested event: {:?}", data);

                let unconfirmed_event = UnconfirmedEvent::new(data.clone(), REQUIRED_CONFIRMATIONS);

                self.blockchain
                    .add_observer(unconfirmed_event.confirmations());

                info!(
                    "Adding PeginRequested event to unconfirmed queue. Event: {:?}",
                    unconfirmed_event
                );

                self.pegin_requested_events.push(unconfirmed_event);
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.pegin_requested_events.is_empty() {
            return Ok(());
        }

        self.blockchain.update(block.clone());

        self.process_confirmed_pegin_requested_events()?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PeginProcessor");

        self.blockchain.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event_processor::EventProcessor, types::PeginRequestedEvent};
    use alloy_primitives::{Address, Bytes, U256};
    use common::{
        msg_broker::{
            broker::{BROKER_SERVER_ID, MockBrokerClientApi},
            types::{FromServer, ToServer},
        },
        test_utils::rsk_block_generator::create_block_and_uncles,
        types::{BlockHash, BlockNumber},
    };
    use mockall::predicate::{eq, function};
    use mockito::mock;
    use primitive_types::H256;
    use serde_json::json;
    use union_contracts::bindings::peg_manager::PegManager::{
        PeginRequested, PrevoutData, RequestPeginTempInfo,
    };

    #[test]
    fn process_new_bitvmx_event_handles_pegin_address_correctly() {
        let _m = mock("POST", "/pegin-address")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"address": "bcrt1test"}"#)
            .create();

        unsafe {
            std::env::set_var("TRANSACTION_DISPATCHER_URL", &mockito::server_url());
        }

        let mut broker = MockBrokerClientApi::new();
        broker
            .expect_send()
            .withf(|dest, msg| {
                *dest == BROKER_SERVER_ID
                    && matches!(msg, ToServer::TemporaryPegInAddressMockedBitVMX(payload)
                        if payload.get("address") == Some(&json!("bcrt1test")))
            })
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(broker);

        let data = json!({
            "btc_reimbursement_pub_key": "0xabc",
            "rootstock_deposit_address": "0xdef",
            "value": 42
        });
        let event = FromServer::FromBitVMX("pegin-address".to_string(), data);

        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_event_fails_on_dispatcher_error() {
        let _m = mockito::mock("POST", "/pegin-address")
            .with_status(500)
            .with_body("Internal Server Error")
            .create();

        unsafe {
            std::env::set_var("TRANSACTION_DISPATCHER_URL", &mockito::server_url());
        }

        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0);

        let mut processor = PeginProcessor::new(broker);

        let data = serde_json::json!({
            "btc_reimbursement_pub_key": "0xabc",
            "rootstock_deposit_address": "0xdef",
            "value": 42
        });

        let event = FromServer::FromBitVMX("pegin-address".to_string(), data);
        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_err());
    }

    #[test]
    fn process_new_bitvmx_event_pegin_requested_does_not_send_response() {
        let _m = mockito::mock("POST", "/register-pegin")
            .with_status(200)
            .with_body(r#"{"result": "ok"}"#)
            .create();

        unsafe {
            std::env::set_var("TRANSACTION_DISPATCHER_URL", &mockito::server_url());
        }

        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0);

        let mut processor = PeginProcessor::new(broker);

        let data = serde_json::json!({
            "block_hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "btc_tx": {
                "version": 1,
                "inputs": [
                    {
                        "tx_id": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "v_out": 0,
                        "sequence": 429496729,
                        "script_sig": "483045022100..."
                    }
                ],
                "outputs": [
                    {
                        "amount": 100000,
                        "script_pub_key": "76a914..."
                    }
                ],
                "lock_time": 0
            },
            "merkle_branch_path": "left-right-left",
            "merkle_branch_hashes": [
                "0x1111111111111111111111111111111111111111111111111111111111111111",
                "0x2222222222222222222222222222222222222222222222222222222222222222"
            ]
        });

        let event = FromServer::FromBitVMX("register-pegin".to_string(), data);
        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_event_pegin_requested_fails_on_dispatcher_error() {
        let _m = mockito::mock("POST", "/register-pegin")
            .with_status(500)
            .with_body("Internal Server Error")
            .create();

        unsafe {
            std::env::set_var("TRANSACTION_DISPATCHER_URL", &mockito::server_url());
        }

        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0);

        let mut processor = PeginProcessor::new(broker);

        let data = serde_json::json!({
            "block_hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "btc_tx": {
                "version": 1,
                "inputs": [
                    {
                        "tx_id": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "v_out": 0,
                        "sequence": 429496729,
                        "script_sig": "483045022100..."
                    }
                ],
                "outputs": [
                    {
                        "amount": 100000,
                        "script_pub_key": "76a914..."
                    }
                ],
                "lock_time": 0
            },
            "merkle_branch_path": "left-right-left",
            "merkle_branch_hashes": [
                "0x1111111111111111111111111111111111111111111111111111111111111111",
                "0x2222222222222222222222222222222222222222222222222222222222222222"
            ]
        });

        let event = FromServer::FromBitVMX("register-pegin".to_string(), data);
        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_err());
    }

    #[test]
    fn process_new_event_pegin_requested_event_and_observer() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: dummy_pegin_requested_event(),
            block_number: BlockNumber::from(123),
            block_hash: BlockHash::from(H256::from([0xaa; 32])),
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());

        assert_eq!(processor.pegin_requested_events.len(), 1);

        let observer_id = processor.pegin_requested_events[0].event_id.to_string();
        assert!(processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_new_event_ignores_unknown_event() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let result = processor.process_new_event(&RskPegManagerEvents::UnknownEvent);
        assert!(result.is_ok());
        assert_eq!(processor.pegin_requested_events.len(), 0);
    }

    #[test]
    fn process_new_block_ignores_if_no_pending_events() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let (block_1, _, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_block_adds_confirmations_but_event_not_confirmed() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let (block_1, _, _) = create_block_and_uncles();

        let req = dummy_pegin_requested_event();
        let event = PeginRequestedEvent {
            inner: req,
            block_number: block_1.number(),
            block_hash: block_1.hash(),
        };

        let unconfirmed = UnconfirmedEvent::new(event.clone(), 5);
        let observer_id = unconfirmed.event_id.to_string();

        processor
            .blockchain
            .add_observer(unconfirmed.confirmations());
        processor.pegin_requested_events.push(unconfirmed);

        // Simulate one new block
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        assert_eq!(processor.pegin_requested_events.len(), 1);
        assert!(processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_new_block_confirms_and_removes_event() {
        let (block_1, _, _) = create_block_and_uncles();

        let req = dummy_pegin_requested_event();
        let event = PeginRequestedEvent {
            inner: req,
            block_number: block_1.number(),
            block_hash: block_1.hash(),
        };

        let unconfirmed = UnconfirmedEvent::new(event.clone(), 1); // confirm after 1 block
        let observer_id = unconfirmed.event_id.to_string();

        let mut broker = MockBrokerClientApi::new();
        let expected_payload = json!(event.inner).to_string();

        broker
            .expect_send()
            .times(1)
            .with(
                eq(BROKER_SERVER_ID),
                function(move |req: &ToServer| {
                    matches!(req, ToServer::ToBitVMX(IncomingBitVMXApiMessages::SetVar(_, variable_name, data))
                        if variable_name == "PeginRequested" &&
                           matches!(data, VariableTypes::String(payload) if *payload == expected_payload))
                }),
            )
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(broker);

        processor
            .blockchain
            .add_observer(unconfirmed.confirmations());
        processor.pegin_requested_events.push(unconfirmed);

        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        // Should have been removed from pending
        assert_eq!(processor.pegin_requested_events.len(), 0);
        assert!(!processor.blockchain.has_observer(&observer_id));
    }

    fn dummy_pegin_requested_event() -> PeginRequested {
        PeginRequested {
            committeeId: U256::from(99),
            requestPeginTxHash: H256::from_low_u64_be(111)
                .as_bytes()
                .try_into()
                .expect("Failed to decode requestPeginTxHash"),
            acceptPeginTxHash: H256::from_low_u64_be(222)
                .as_bytes()
                .try_into()
                .expect("Failed to decode acceptPeginTxHash"),
            vout: 1,
            streamId: 42,
            packetNumber: 33,
            requestPeginInfo: RequestPeginTempInfo {
                rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                    .parse::<alloy_primitives::Address>()
                    .expect("Invalid address"),
                btcReimbursementPubKey: H256::from_low_u64_be(103991732982)
                    .as_bytes()
                    .try_into()
                    .expect("Failed to decode key"),
                acceptPeginSignatureHash: H256::from_low_u64_be(4444444)
                    .as_bytes()
                    .try_into()
                    .expect("Failed to decode hash"),
            },
            prevoutData: PrevoutData {
                value: 1000,
                scriptPubKey: alloy_primitives::Bytes::from("0x1234567890abcdef"),
            },
            acceptPeginSignatureMessage: alloy_primitives::Bytes::from("0xabcdef0123456789"),
        }
    }
}
