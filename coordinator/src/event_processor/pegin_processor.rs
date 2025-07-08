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
    types::{RskBlockAndUncles, TxHash},
};
use log::info;
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, env, rc::Rc};
use union_contracts::bindings::peg_manager::PegManager::{PeginAccepted, PeginRequested};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct PeginEvent<T: Clone> {
    data: EventWithBlock<T>,
    confirmations: Rc<RefCell<BlockConfirmations>>,
    is_handled: bool,
}

impl<T: Clone> PeginEvent<T> {
    fn new(data: EventWithBlock<T>, confirmations: BlockConfirmations) -> Self {
        let rc_confirmations = Rc::new(RefCell::new(confirmations));

        Self {
            data,
            confirmations: rc_confirmations,
            is_handled: false,
        }
    }

    fn data(&self) -> T {
        self.data.inner.clone()
    }

    fn is_confirmed(&self) -> bool {
        self.confirmations.borrow().is_confirmed()
    }

    fn confirmations(&self) -> Rc<RefCell<BlockConfirmations>> {
        self.confirmations.clone()
    }

    fn is_handled(&self) -> bool {
        self.is_handled
    }

    fn mark_handled(&mut self) {
        self.is_handled = true
    }
}

#[derive(Debug)]
struct PeginEventState {
    pegin_flow_id: Uuid,
    pegin_requested: PeginEvent<PeginRequested>,
    pegin_accepted: Option<PeginEvent<PeginAccepted>>,
}

impl PeginEventState {
    fn new(pegin_flow_id: Uuid, pegin_requested: PeginEvent<PeginRequested>) -> Self {
        Self {
            pegin_flow_id,
            pegin_requested,
            pegin_accepted: None,
        }
    }

    fn pegin_flow_id(&self) -> Uuid {
        self.pegin_flow_id
    }

    fn pegin_requested(&self) -> &PeginEvent<PeginRequested> {
        &self.pegin_requested
    }

    fn pegin_accepted(&self) -> Option<&PeginEvent<PeginAccepted>> {
        self.pegin_accepted.as_ref()
    }

    fn pegin_requested_mut(&mut self) -> &mut PeginEvent<PeginRequested> {
        &mut self.pegin_requested
    }

    fn pegin_accepted_mut(&mut self) -> Option<&mut PeginEvent<PeginAccepted>> {
        self.pegin_accepted.as_mut()
    }

    fn set_pegin_accepted(&mut self, pegin_accepted: PeginEvent<PeginAccepted>) {
        self.pegin_accepted = Some(pegin_accepted);
    }
}

pub struct PeginProcessor<T: BrokerClientApi> {
    http_client: Client,
    bitvmx_broker: T,
    blockchain: BlockchainView,
    tracker: HashMap<TxHash, PeginEventState>,
}

impl<T: BrokerClientApi> PeginProcessor<T> {
    pub fn new(bitvmx_broker: T) -> Self {
        Self {
            http_client: Client::new(),
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            tracker: HashMap::new(),
        }
    }

    /// Inserts a `pegin_requested` event. Fails if one already exists for the same tx_hash.
    fn insert_pegin_requested(
        &mut self,
        pegin_flow_id: Uuid,
        event: PeginEvent<PeginRequested>,
    ) -> Result<()> {
        let tx_hash: TxHash = event.data.inner.acceptPeginTxHash.into();

        if self.tracker.contains_key(&tx_hash) {
            bail!("PeginRequested already exists for tx_hash: {:?}", tx_hash);
        }

        self.tracker
            .insert(tx_hash, PeginEventState::new(pegin_flow_id, event));

        Ok(())
    }

    /// Inserts a `pegin_accepted` event. Fails if no corresponding `pegin_requested` exists.
    fn insert_pegin_accepted(&mut self, event: PeginEvent<PeginAccepted>) -> Result<()> {
        let tx_hash: TxHash = event.data.inner.acceptPeginTxHash.into();

        match self.tracker.get_mut(&tx_hash) {
            Some(state) => {
                state.set_pegin_accepted(event);
                Ok(())
            }
            None => {
                bail!("PeginRequested cannot be found for tx_hash: {:?}", tx_hash);
            }
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
            "accept-pegin" => {
                // No response needed for accept-pegin
                Ok(true)
            }
            _ => bail!("Unsupported method name for BitVMX response: {}", method),
        }
    }

    fn process_unhandled_confirmed_pegin_requested_events(&mut self) -> Result<()> {
        let mut events = Vec::new();

        for (tx_hash, state) in &self.tracker {
            let event = state.pegin_requested();
            if event.is_confirmed() && !event.is_handled() {
                events.push((tx_hash.clone(), state.pegin_flow_id(), event.data().clone()));
            }
        }

        for (tx_hash, pegin_flow_id, data) in events {
            match self.handle_confirmed_event(pegin_flow_id, "PeginRequested", &data) {
                Ok(_) => {
                    if let Some(state) = self.tracker.get_mut(&tx_hash) {
                        state.pegin_requested_mut().mark_handled();
                        self.blockchain
                            .remove_observer(pegin_flow_id.to_string().as_str());
                        info!(
                            "Successfully processed confirmed PeginRequested event: {}",
                            pegin_flow_id
                        );
                    } else {
                        bail!(
                            "Tracker missing expected state for tx_hash {} (flow_id: {}) when handling PeginRequested",
                            tx_hash,
                            pegin_flow_id
                        );
                    }
                }
                Err(e) => {
                    bail!(
                        "Error processing confirmed PeginRequested event (tx_hash: {}, flow_id: {}): {}",
                        tx_hash,
                        pegin_flow_id,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    fn process_unhandled_confirmed_pegin_accepted_events(&mut self) -> Result<()> {
        let mut events = Vec::new();

        for (tx_hash, state) in &self.tracker {
            if let Some(event) = state.pegin_accepted() {
                if event.is_confirmed() && !event.is_handled() {
                    events.push((tx_hash.clone(), state.pegin_flow_id(), event.data().clone()));
                }
            }
        }

        for (tx_hash, pegin_flow_id, data) in events {
            match self.handle_confirmed_event(pegin_flow_id, "PeginAccepted", &data) {
                Ok(_) => {
                    if let Some(state) = self.tracker.get_mut(&tx_hash) {
                        if let Some(pegin_accepted) = state.pegin_accepted_mut() {
                            pegin_accepted.mark_handled();
                            self.blockchain
                                .remove_observer(pegin_flow_id.to_string().as_str());
                            info!(
                                "Successfully processed confirmed PeginAccepted event: {}",
                                pegin_flow_id
                            );
                        } else {
                            bail!(
                                "Expected PeginAccepted event not present for tx_hash {} (flow_id: {})",
                                tx_hash,
                                pegin_flow_id
                            );
                        }
                    } else {
                        bail!(
                            "Tracker missing expected state for tx_hash {} (flow_id: {}) when handling PeginAccepted",
                            tx_hash,
                            pegin_flow_id
                        );
                    }
                }
                Err(e) => {
                    bail!(
                        "Error processing confirmed PeginAccepted event (tx_hash: {}, flow_id: {}): {}",
                        tx_hash,
                        pegin_flow_id,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    fn handle_confirmed_event<E: Serialize>(
        &self,
        pegin_flow_id: Uuid,
        variable_name: &str,
        data: &E,
    ) -> Result<()> {
        let data = serde_json::to_string(data)?;

        self.bitvmx_broker.send(
            BROKER_SERVER_ID,
            ToServer::ToBitVMX(IncomingBitVMXApiMessages::SetVar(
                pegin_flow_id,
                variable_name.to_string(),
                VariableTypes::String(data),
            )),
        )?;

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
                if matches!(
                    method.as_str(),
                    "pegin-address" | "register-pegin" | "accept-pegin"
                ) =>
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

                let pegin_flow_id = Uuid::new_v4();

                let confirmations = BlockConfirmations::new(
                    pegin_flow_id.to_string(),
                    data.block_number,
                    REQUIRED_CONFIRMATIONS,
                );

                let pegin_requested = PeginEvent::new(data.clone(), confirmations);

                self.blockchain
                    .add_observer(pegin_requested.confirmations());

                info!(
                    "Adding PeginRequested event to pegin event tracker. Event: {:?}",
                    pegin_requested
                );

                self.insert_pegin_requested(pegin_flow_id, pegin_requested)?;
            }
            RskPegManagerEvents::PeginAccepted(data) => {
                info!("Handling PeginAccepted event: {:?}", data);

                let tx_hash: TxHash = data.inner.acceptPeginTxHash.into();
                let Some(state) = self.tracker.get(&tx_hash) else {
                    bail!(
                        "Received PeginAccepted for unknown acceptPeginTxHash: {:?}",
                        tx_hash
                    );
                };

                let pegin_flow_id = state.pegin_flow_id();

                let confirmations = BlockConfirmations::new(
                    pegin_flow_id.to_string(),
                    data.block_number,
                    REQUIRED_CONFIRMATIONS,
                );

                let pegin_accepted = PeginEvent::new(data.clone(), confirmations);

                self.blockchain.add_observer(pegin_accepted.confirmations());

                info!(
                    "Adding PeginAccepted event to pegin event tracker. Event: {:?}",
                    pegin_accepted
                );

                self.insert_pegin_accepted(pegin_accepted)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.tracker.is_empty() {
            return Ok(());
        }

        self.blockchain.update(block.clone());

        self.process_unhandled_confirmed_pegin_requested_events()?;
        self.process_unhandled_confirmed_pegin_accepted_events()?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PeginProcessor");

        self.blockchain.clear();
        self.tracker.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event_processor::EventProcessor,
        types::{PeginAcceptedEvent, PeginRequestedEvent},
    };
    use alloy_primitives::{Address, Bytes, FixedBytes, U256};
    use common::{
        msg_broker::{
            broker::{BROKER_SERVER_ID, MockBrokerClientApi},
            types::{FromServer, ToServer},
        },
        test_utils::rsk_block_generator::create_block_and_uncles,
        types::BlockHash,
    };
    use mockall::predicate::{eq, function};
    use mockito::mock;
    use primitive_types::H256;
    use serde_json::json;
    use union_contracts::bindings::peg_manager::PegManager::{
        PeginRequested, PrevoutData, RequestPeginTempInfo, StreamPosition,
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
    fn process_new_bitvmx_pegin_accepted_event_does_not_send_response() {
        let _m = mockito::mock("POST", "/accept-pegin")
            .with_status(200)
            .with_body(r#"{"result": "ok"}"#)
            .create();

        unsafe {
            std::env::set_var("TRANSACTION_DISPATCHER_URL", &mockito::server_url());
        }

        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0); // Should not send anything

        let mut processor = PeginProcessor::new(broker);

        let data = serde_json::json!({
            "block_hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "btc_tx": {
                "version": 1,
                "inputs": [{
                    "tx_id": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "v_out": 0,
                    "sequence": 429496729,
                    "script_sig": "483045022100..."
                }],
                "outputs": [{
                    "amount": 100000,
                    "script_pub_key": "76a914..."
                }],
                "lock_time": 0
            },
            "merkle_branch_path": "left-right-left",
            "merkle_branch_hashes": [
                "0x1111111111111111111111111111111111111111111111111111111111111111",
                "0x2222222222222222222222222222222222222222222222222222222222222222"
            ]
        });

        let event = FromServer::FromBitVMX("accept-pegin".to_string(), data);
        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_pegin_accepted_event_fails_on_dispatcher_error() {
        let _m = mockito::mock("POST", "/accept-pegin")
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
                "inputs": [{
                    "tx_id": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "v_out": 0,
                    "sequence": 429496729,
                    "script_sig": "483045022100..."
                }],
                "outputs": [{
                    "amount": 100000,
                    "script_pub_key": "76a914..."
                }],
                "lock_time": 0
            },
            "merkle_branch_path": "left-right-left",
            "merkle_branch_hashes": [
                "0x1111111111111111111111111111111111111111111111111111111111111111",
                "0x2222222222222222222222222222222222222222222222222222222222222222"
            ]
        });

        let event = FromServer::FromBitVMX("accept-pegin".to_string(), data);
        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_err());
    }

    #[test]
    fn process_new_event_pegin_requested_event_and_observer() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let pegin_requested = dummy_pegin_requested_event();
        let tx_hash: TxHash = pegin_requested.acceptPeginTxHash.into();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 123.into(),
            block_hash: BlockHash::from(H256::from([0xaa; 32])),
            removed: false,
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);

        let observer_id = processor
            .tracker
            .get(&tx_hash)
            .unwrap()
            .pegin_flow_id()
            .to_string();
        assert!(processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_new_event_registers_pegin_accepted_event_and_observer() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let pegin_requested = dummy_pegin_requested_event();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 122.into(),
            block_hash: BlockHash::from(H256::from([0xba; 32])),
            removed: false,
        });
        let result = processor.process_new_event(&event);
        assert!(result.is_ok());

        let pegin_accepted = dummy_pegin_accepted_event();
        let tx_hash: TxHash = pegin_accepted.acceptPeginTxHash.into();
        let event = RskPegManagerEvents::PeginAccepted(PeginAcceptedEvent {
            inner: dummy_pegin_accepted_event(),
            block_number: 456.into(),
            block_hash: BlockHash::from(H256::from([0xbb; 32])),
            removed: false,
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);

        let observer_id = processor
            .tracker
            .get(&tx_hash)
            .unwrap()
            .pegin_flow_id()
            .to_string();
        assert!(processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_new_event_ignores_unknown_event() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let result = processor.process_new_event(&RskPegManagerEvents::UnknownEvent);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 0);
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
    fn process_new_block_adds_confirmations_for_register_pegin_but_event_not_confirmed() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let (block_1, _, _) = create_block_and_uncles();

        let pegin_requested = dummy_pegin_requested_event();
        let event = PeginRequestedEvent {
            inner: pegin_requested,
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
        };

        let pegin_flow_id = Uuid::new_v4();
        let confirmations = BlockConfirmations::new(
            pegin_flow_id.to_string(),
            block_1.number(),
            REQUIRED_CONFIRMATIONS,
        );
        let pegin_event = PeginEvent::new(event.clone(), confirmations);

        processor
            .blockchain
            .add_observer(pegin_event.confirmations());
        let _ = processor.insert_pegin_requested(pegin_flow_id, pegin_event);

        // Simulate one new block
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);
        assert!(
            processor
                .blockchain
                .has_observer(&pegin_flow_id.to_string())
        );
    }

    #[test]
    fn process_new_block_confirms_and_removes_event() {
        let (block_1, _, _) = create_block_and_uncles();

        let pegin_requested = dummy_pegin_requested_event();
        let event = PeginRequestedEvent {
            inner: pegin_requested.clone(),
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
        };

        let pegin_flow_id = Uuid::new_v4();
        let confirmations = BlockConfirmations::new(
            pegin_flow_id.to_string(),
            block_1.number(),
            1, // confirm after 1 block
        );
        let pegin_event = PeginEvent::new(event.clone(), confirmations);

        let mut broker = MockBrokerClientApi::new();
        let expected_payload = json!(event.inner);

        broker
            .expect_send()
            .times(1)
            .with(
    eq(BROKER_SERVER_ID),
    function(move |req: &ToServer| {
        matches!(
            req,
            ToServer::ToBitVMX(IncomingBitVMXApiMessages::SetVar(_, variable_name, VariableTypes::String(actual)))
                if variable_name == "PeginRequested"
                && serde_json::from_str::<Value>(actual).ok() == Some(expected_payload.clone())
        )
    }),
)
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(broker);

        processor
            .blockchain
            .add_observer(pegin_event.confirmations());
        let _ = processor.insert_pegin_requested(pegin_flow_id, pegin_event);

        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);
        assert!(
            !processor
                .blockchain
                .has_observer(&pegin_flow_id.to_string())
        );
    }

    #[test]
    fn process_new_block_adds_confirmations_for_pegin_accepted_event_not_confirmed() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(broker);

        let (block_1, block_2, _) = create_block_and_uncles();

        let pegin_requested = dummy_pegin_requested_event();
        let pegin_requested_event = PeginRequestedEvent {
            inner: pegin_requested.clone(),
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
        };

        let pegin_accepted = dummy_pegin_accepted_event();
        let pegin_accepted_event = PeginAcceptedEvent {
            inner: pegin_accepted,
            block_number: block_2.number(),
            block_hash: block_2.hash(),
            removed: false,
        };

        let pegin_flow_id = Uuid::new_v4();

        let pegin_requested_confirmations = BlockConfirmations::new(
            pegin_flow_id.to_string(),
            block_1.number(),
            REQUIRED_CONFIRMATIONS,
        );
        let pegin_accepted_confirmations = BlockConfirmations::new(
            pegin_flow_id.to_string(),
            block_2.number(),
            REQUIRED_CONFIRMATIONS,
        );

        let pegin_requested_event =
            PeginEvent::new(pegin_requested_event.clone(), pegin_requested_confirmations);
        let pegin_accepted_event =
            PeginEvent::new(pegin_accepted_event.clone(), pegin_accepted_confirmations);

        processor
            .blockchain
            .add_observer(pegin_accepted_event.confirmations());

        let _ = processor.insert_pegin_requested(pegin_flow_id, pegin_requested_event);
        let _ = processor.insert_pegin_accepted(pegin_accepted_event);

        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);
        assert!(
            processor
                .blockchain
                .has_observer(&pegin_flow_id.to_string())
        );
    }

    #[test]
    fn process_new_block_confirms_and_removes_pegin_accepted_event() {
        let mut broker = MockBrokerClientApi::new();
        broker
            .expect_send()
            .withf(|dest, msg| {
                *dest == BROKER_SERVER_ID
                    && matches!(
                        msg,
                        ToServer::ToBitVMX(
                            IncomingBitVMXApiMessages::SetVar(_, name, VariableTypes::String(_))
                        ) if name == "PeginAccepted"
                    )
            })
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(broker);

        let pegin_requested = dummy_pegin_requested_event();
        let pegin_requested_event = PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 99.into(),
            block_hash: BlockHash::from(H256::from_low_u64_be(122)),
            removed: false,
        };

        let pegin_accepted = dummy_pegin_accepted_event();
        let pegin_accepted_event = PeginAcceptedEvent {
            inner: pegin_accepted,
            block_number: 100.into(),
            block_hash: BlockHash::from(H256::from_low_u64_be(123)),
            removed: false,
        };

        let pegin_flow_id = Uuid::new_v4();

        let pegin_requested_confirmations =
            BlockConfirmations::new(pegin_flow_id.to_string(), 99.into(), 0);
        let pegin_accepted_confirmations =
            BlockConfirmations::new(pegin_flow_id.to_string(), 100.into(), 1);

        let mut pegin_requested_event =
            PeginEvent::new(pegin_requested_event.clone(), pegin_requested_confirmations);
        pegin_requested_event.mark_handled(); // assumes already handled
        let pegin_accepted_event =
            PeginEvent::new(pegin_accepted_event.clone(), pegin_accepted_confirmations);

        processor
            .blockchain
            .add_observer(pegin_accepted_event.confirmations());
        let _ = processor.insert_pegin_requested(pegin_flow_id, pegin_requested_event);
        let _ = processor.insert_pegin_accepted(pegin_accepted_event);

        let (block_1, _, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);
        assert!(
            !processor
                .blockchain
                .has_observer(&pegin_flow_id.to_string())
        );
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

    fn dummy_pegin_accepted_event() -> PeginAccepted {
        PeginAccepted {
            blockHash: FixedBytes::<32>::from_slice(H256::from_low_u64_be(1).as_bytes()),
            acceptPeginTxHash: FixedBytes::<32>::from_slice(H256::from_low_u64_be(222).as_bytes()),
            peginRequestTxHash: FixedBytes::<32>::from_slice(H256::from_low_u64_be(3).as_bytes()),
            vout: 0,
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 1.into(),
            },
            speedUpPubKey: FixedBytes::<32>::from_slice(
                H256::from_low_u64_be(103991732982).as_bytes(),
            ),
            rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                .parse::<Address>()
                .expect("Invalid address"),
            rbtcAmount: U256::from(12345678),
            utxoScriptPubKey: Bytes::from("0xabcdef0123456789"),
        }
    }
}
