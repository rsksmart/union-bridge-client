use crate::{
    config::REQUIRED_CONFIRMATIONS,
    event_processor::{
        EventProcessor,
        blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView},
    },
    types::{EventWithBlock, RskPegManagerEvents},
};
use anyhow::{Context, Result, bail};
use common::{
    msg_broker::{
        bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, VariableTypes},
        broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi},
    },
    runtime_sync::RuntimeSync,
    types::{RskBlockAndUncles, TxHash},
};
use log::{error, info};
use serde::Serialize;
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};
use transaction_dispatcher::{
    rsk_gateway::RskContractsGatewayApi,
    types::{AcceptPegInInput, RegisterPegInInput},
};
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

    fn is_confirmed(&self) -> bool {
        self.confirmations.borrow().is_confirmed()
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
}

pub struct PeginProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    rt_sync: RuntimeSync,
    contracts: Arc<CG>,
    bitvmx_broker: Arc<BC>,
    blockchain: BlockchainView,
    tracker: HashMap<TxHash, PeginEventState>,
}

impl<CG, BC> PeginProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(rt_sync: RuntimeSync, contracts: Arc<CG>, bitvmx_broker: Arc<BC>) -> Self {
        Self {
            rt_sync,
            contracts,
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            tracker: HashMap::new(),
        }
    }

    fn track_pegin_requested(
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

    fn track_pegin_accepted(&mut self, event: PeginEvent<PeginAccepted>) -> Result<()> {
        let tx_hash: TxHash = event.data.inner.acceptPeginTxHash.into();

        match self.tracker.get_mut(&tx_hash) {
            Some(state) => {
                state.pegin_accepted = Some(event);
                Ok(())
            }
            None => {
                bail!("PeginRequested cannot be found for tx_hash: {:?}", tx_hash);
            }
        }
    }

    fn untrack_pegin_requested(&mut self, event: EventWithBlock<PeginRequested>) -> Result<()> {
        let tx_hash: TxHash = event.inner.acceptPeginTxHash.into();

        match self.tracker.remove(&tx_hash) {
            Some(state) => {
                if state.pegin_accepted.is_some() {
                    bail!(
                        "PeginAccepted found while trying to remove PeginRequested event. This should never occur."
                    );
                }

                let confirmations = state.pegin_requested.confirmations.borrow();
                let observer_id = confirmations.get_id();
                self.blockchain.remove_observer(observer_id.as_str());

                info!(
                    "Untracked PeginRequested event. tx_hash: {:?}, pegin_flow_id: {}",
                    tx_hash, state.pegin_flow_id
                );

                Ok(())
            }
            None => bail!(
                "Expected to untrack PeginRequested event but no entry found for tx_hash: {:?}",
                tx_hash
            ),
        }
    }

    fn untrack_pegin_accepted(&mut self, event: EventWithBlock<PeginAccepted>) -> Result<()> {
        let tx_hash: TxHash = event.inner.acceptPeginTxHash.into();

        match self.tracker.get_mut(&tx_hash) {
            Some(state) => {
                if let Some(pegin_accepted) = &state.pegin_accepted {
                    let confirmations = pegin_accepted.confirmations.borrow();
                    let observer_id = confirmations.get_id();
                    self.blockchain.remove_observer(observer_id.as_str());
                } else {
                    bail!(
                        "Trying to untrack PeginAccepted event, but tracker entry for tx_hash: {:?} has no PeginAccepted event",
                        tx_hash
                    );
                }

                state.pegin_accepted = None;

                info!(
                    "Untracked PeginAccepted event. tx_hash: {:?}, pegin_flow_id: {}",
                    tx_hash, state.pegin_flow_id
                );

                Ok(())
            }
            None => bail!(
                "Expected to untrack PeginAccepted event but no entry found for tx_hash: {:?}",
                tx_hash
            ),
        }
    }

    fn process_unhandled_confirmed_pegin_requested_events(&mut self) -> Result<()> {
        for (tx_hash, state) in self.tracker.iter_mut() {
            let flow_id = state.pegin_flow_id;

            let event = &mut state.pegin_requested;
            if !event.is_confirmed() || event.is_handled {
                continue;
            }

            Self::send_to_bitvmx(
                &self.bitvmx_broker,
                flow_id,
                "PeginRequested",
                &event.data.inner,
            )
            .context(format!(
                "Error processing confirmed PeginRequested event (tx_hash: {}, flow_id: {})",
                tx_hash, flow_id
            ))?;

            event.mark_handled();

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            info!(
                "Successfully processed confirmed PeginRequested event: {}",
                flow_id
            );
        }

        Ok(())
    }

    fn process_unhandled_confirmed_pegin_accepted_events(&mut self) -> Result<()> {
        let mut to_remove = Vec::new();

        for (tx_hash, state) in self.tracker.iter_mut() {
            let flow_id = state.pegin_flow_id;

            let event = match state.pegin_accepted.as_mut() {
                None => continue,
                Some(event) if !event.is_confirmed() || event.is_handled => continue,
                Some(event) => event,
            };

            Self::send_to_bitvmx(
                &self.bitvmx_broker,
                flow_id,
                "PeginAccepted",
                &event.data.inner,
            )
            .context(format!(
                "Error processing confirmed PeginAccepted event (tx_hash: {}, flow_id: {})",
                tx_hash, flow_id
            ))?;

            event.mark_handled();

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            info!(
                "Successfully processed confirmed PeginAccepted event: {}",
                flow_id
            );

            to_remove.push((*tx_hash, flow_id));
        }

        // Pegin completed so we can remove the state in tracker
        for (tx_hash, flow_id) in to_remove {
            info!(
                "Pegin completed. Removing pegin event state. tx_hash: {:?}, flow_id: {}",
                tx_hash, flow_id
            );
            self.tracker.remove(&tx_hash);
        }

        Ok(())
    }

    fn send_to_union_bridge(&self, method_name: &str, json_value: &Value) -> Result<()> {
        info!(
            "Dispatching transaction to union bridge. Method: '{}', Payload: {}",
            method_name, json_value
        );

        match method_name {
            "register-pegin" => {
                let input: RegisterPegInInput = serde_json::from_value(json_value.clone())
                    .context("Failed to deserialize RegisterPegInInput")?;

                match self
                    .rt_sync
                    .run(async { self.contracts.register_peg_in_request(input).await })
                {
                    Ok(_) => {
                        info!("Successfully called '{}'", method_name);
                        Ok(())
                    }
                    Err(domain_err) => {
                        error!("Error calling '{}': {:?}", method_name, domain_err);
                        Err(domain_err.into())
                    }
                }
            }

            "accept-pegin" => {
                let input: AcceptPegInInput = serde_json::from_value(json_value.clone())
                    .context("Failed to deserialize AcceptPegInInput")?;

                match self
                    .rt_sync
                    .run(async { self.contracts.accept_peg_in_request(input).await })
                {
                    Ok(_) => {
                        info!("Successfully called '{}'", method_name);
                        Ok(())
                    }
                    Err(domain_err) => {
                        error!("Error calling '{}': {:?}", method_name, domain_err);
                        Err(domain_err.into())
                    }
                }
            }

            _ => bail!("Unsupported method: {}", method_name),
        }
    }

    fn send_to_bitvmx<E: Serialize>(
        bitvmx_broker: &BC,
        pegin_flow_id: Uuid,
        variable_name: &str,
        data: &E,
    ) -> Result<()> {
        let data = serde_json::to_string(data)?;

        bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SetVar(
                pegin_flow_id,
                variable_name.to_string(),
                VariableTypes::String(data),
            ),
        )?;

        Ok(())
    }
}

impl<CG, BC> EventProcessor for PeginProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::Variable(
                pegin_flow_id,
                method,
                VariableTypes::String(data),
            ) if matches!(method.as_str(), "register-pegin" | "accept-pegin") => {
                info!(
                    "Handling BitVMX Variable Event. Flow Id: {}, Method: {}, Payload: {:?}",
                    pegin_flow_id, method, data
                );

                let json_data: Value = serde_json::from_str(data)?;

                self.send_to_union_bridge(method, &json_data)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::PeginRequested(data) => {
                if data.removed {
                    info!("Handling PeginRequested removed event: {:?}", data);

                    return self.untrack_pegin_requested(data.clone());
                }

                info!("Handling PeginRequested event: {:?}", data);

                let pegin_flow_id = Uuid::new_v4();
                let observer_id = format!("pegin_requested-{}", pegin_flow_id);

                let confirmations =
                    BlockConfirmations::new(observer_id, data.block_number, REQUIRED_CONFIRMATIONS);

                let pegin_requested = PeginEvent::new(data.clone(), confirmations);

                self.blockchain
                    .add_observer(pegin_requested.confirmations.clone());

                info!(
                    "Adding PeginRequested event to pegin event tracker. Event: {:?}",
                    pegin_requested
                );

                self.track_pegin_requested(pegin_flow_id, pegin_requested)?;
            }
            RskPegManagerEvents::PeginAccepted(data) => {
                if data.removed {
                    info!("Handling PeginAccepted removed event: {:?}", data);

                    return self.untrack_pegin_accepted(data.clone());
                }

                info!("Handling PeginAccepted event: {:?}", data);

                let tx_hash: TxHash = data.inner.acceptPeginTxHash.into();
                let Some(state) = self.tracker.get(&tx_hash) else {
                    bail!(
                        "Received PeginAccepted for unknown acceptPeginTxHash: {:?}",
                        tx_hash
                    );
                };

                let observer_id = format!("pegin_accepted-{}", state.pegin_flow_id);
                let confirmations =
                    BlockConfirmations::new(observer_id, data.block_number, REQUIRED_CONFIRMATIONS);

                let pegin_accepted = PeginEvent::new(data.clone(), confirmations);

                self.blockchain
                    .add_observer(pegin_accepted.confirmations.clone());

                info!(
                    "Adding PeginAccepted event to pegin event tracker. Event: {:?}",
                    pegin_accepted
                );

                self.track_pegin_accepted(pegin_accepted)?;
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
        coordinator::tests::MockRskContractsGatewayApi,
        event_processor::EventProcessor,
        types::{PeginAcceptedEvent, PeginRequestedEvent},
    };
    use alloy_primitives::{Address, Bytes, FixedBytes, U256};
    use common::{
        msg_broker::broker::{BROKER_SERVER_ID, MockBrokerClientApi},
        test_utils::rsk_block_generator::create_block_and_uncles,
        types::BlockHash,
    };
    use mockall::predicate::{eq, function};
    use primitive_types::H256;
    use serde_json::json;
    use transaction_dispatcher::{
        rsk_gateway::DomainErrors,
        types::{AcceptPegInOutput, RegisterPegInOutput},
    };
    use union_contracts::bindings::peg_manager::PegManager::{
        PeginRequested, PrevoutData, RequestPeginTempInfo, StreamPosition,
    };

    #[test]
    fn process_new_bitvmx_event_pegin_requested_does_not_send_response() {
        // Prepare the mocked contracts gateway
        let mut contracts = MockRskContractsGatewayApi::new();
        let expected_receipt = RegisterPegInOutput {
            transaction_hash: "0x4e3f8a2d39c1b872b77e8a5c9a24be8f1d489ea7cf2d38375f18b5b54e7df662"
                .to_string(),
            success: true,
        };
        contracts
            .expect_register_peg_in_request()
            .times(1)
            .returning(move |_| Ok(expected_receipt.clone()));

        // Prepare broker and assert it doesn't send anything
        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(rt_sync, contracts.into(), broker.into());

        // Simulate event payload
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

        let payload = serde_json::to_string(&data).unwrap();
        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "register-pegin".to_string(),
            VariableTypes::String(payload),
        );

        // Run and assert
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_event_pegin_requested_fails_on_dispatcher_error() {
        // Prepare a mocked contracts gateway that simulates a failure
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts
            .expect_register_peg_in_request()
            .times(1)
            .returning(|_| Err(DomainErrors::UnknownContractError("simulated error".into())));

        // Prepare broker and assert it doesn't send anything
        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(rt_sync, contracts.into(), broker.into());

        // Simulate payload
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

        let payload = serde_json::to_string(&data).unwrap();
        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "register-pegin".to_string(),
            VariableTypes::String(payload),
        );

        let result = processor.process_new_bitvmx_event(&event);

        // We expect an error due to contract dispatch failure
        assert!(result.is_err());
    }

    #[test]
    fn process_new_bitvmx_pegin_accepted_event_does_not_send_response() {
        // Prepare the mocked contracts gateway
        let mut contracts = MockRskContractsGatewayApi::new();
        let expected_receipt = AcceptPegInOutput {
            transaction_hash: "0x7e8f27d21c8a0cfebfd2c647db4687e51eae3eaecdbf9f247c9057be682176a3"
                .to_string(),
            success: true,
        };
        contracts
            .expect_accept_peg_in_request()
            .times(1)
            .returning(move |_| Ok(expected_receipt.clone()));

        // Prepare broker and assert it doesn't send anything
        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0); // Should not send anything

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(rt_sync, contracts.into(), broker.into());

        // Simulate event payload
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

        let payload = serde_json::to_string(&data).unwrap();
        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "accept-pegin".to_string(),
            VariableTypes::String(payload),
        );

        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_pegin_accepted_event_fails_on_dispatcher_error() {
        // Set up the mocked contracts gateway with an error
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts
            .expect_accept_peg_in_request()
            .times(1)
            .returning(|_| Err(DomainErrors::UnknownContractError("simulated error".into())));

        // Set up a broker that should not be called
        let mut broker = MockBrokerClientApi::new();
        broker.expect_send().times(0);

        // Runtime and processor initialization
        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(rt_sync, contracts.into(), broker.into());

        // Payload
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

        let payload = serde_json::to_string(&data).unwrap();
        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "accept-pegin".to_string(),
            VariableTypes::String(payload),
        );

        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn process_new_event_pegin_requested_event_and_observer() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let tx_hash: TxHash = pegin_requested.acceptPeginTxHash.into();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 123.into(),
            block_hash: BlockHash::from(H256::from([0xaa; 32])),
            removed: false,
            tx_hash: tx_hash.clone(),
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);

        let observer_id = processor
            .tracker
            .get(&tx_hash)
            .map(|state| state.pegin_requested.confirmations.borrow().get_id())
            .unwrap();
        assert!(processor.blockchain.has_observer(observer_id.as_str()));
    }

    #[test]
    fn process_removed_pegin_requested_event() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let tx_hash: TxHash = pegin_requested.acceptPeginTxHash.into();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested.clone(),
            block_number: 123.into(),
            block_hash: BlockHash::from(H256::from([0xaa; 32])),
            removed: false,
            tx_hash: tx_hash.clone(),
        });

        let result = processor.process_new_event(&event);
        let observer_id = processor
            .tracker
            .get(&tx_hash)
            .map(|state| state.pegin_requested.confirmations.borrow().get_id())
            .unwrap();
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 1);
        assert!(processor.blockchain.has_observer(observer_id.as_str()));

        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested.clone(),
            block_number: 123.into(),
            block_hash: BlockHash::from(H256::from([0xaa; 32])),
            removed: true, // event is removed,
            tx_hash: tx_hash.clone(),
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 0);
        assert!(!processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_new_event_pegin_accepted_event_and_observer() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 122.into(),
            block_hash: BlockHash::from(H256::from([0xba; 32])),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
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
            tx_hash: tx_hash.clone(),
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 1);

        let observer_id = processor
            .tracker
            .get(&tx_hash)
            .and_then(|state| state.pegin_accepted.as_ref())
            .map(|accepted| accepted.confirmations.borrow().get_id())
            .unwrap();
        assert!(processor.blockchain.has_observer(observer_id.as_str()));
    }

    #[test]
    fn process_removed_event_pegin_accepted_event() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 122.into(),
            block_hash: BlockHash::from(H256::from([0xba; 32])),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(9)),
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
            tx_hash: tx_hash.clone(),
        });

        let result = processor.process_new_event(&event);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 1);

        let event = RskPegManagerEvents::PeginAccepted(PeginAcceptedEvent {
            inner: dummy_pegin_accepted_event(),
            block_number: 456.into(),
            block_hash: BlockHash::from(H256::from([0xbb; 32])),
            removed: true, // event is removed
            tx_hash: TxHash::from(H256::from_low_u64_be(10)),
        });

        let result = processor.process_new_event(&event);
        let observer_id = processor
            .tracker
            .get(&tx_hash)
            .unwrap()
            .pegin_flow_id
            .to_string();
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 1);
        assert!(!processor.blockchain.has_observer(&observer_id));
        assert!(
            processor
                .tracker
                .get(&tx_hash)
                .unwrap()
                .pegin_accepted
                .is_none()
        );
    }

    #[test]
    fn process_new_event_ignores_unknown_event() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let result = processor.process_new_event(&RskPegManagerEvents::UnknownEvent);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 0);
    }

    #[test]
    fn process_new_block_ignores_if_no_pending_events() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let (block_1, _, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_block_adds_confirmations_for_register_pegin_but_event_not_confirmed() {
        let broker = MockBrokerClientApi::new();
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let (block_1, _, _) = create_block_and_uncles();

        let pegin_requested = dummy_pegin_requested_event();
        let event = PeginRequestedEvent {
            inner: pegin_requested,
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(9)),
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
            .add_observer(pegin_event.confirmations.clone());
        let _ = processor.track_pegin_requested(pegin_flow_id, pegin_event);

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
            tx_hash: TxHash::from(H256::from_low_u64_be(9)),
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
    function(move |req: &IncomingBitVMXApiMessages| {
        matches!(
            req,
            IncomingBitVMXApiMessages::SetVar(_, variable_name, VariableTypes::String(actual))
                if variable_name == "PeginRequested"
                && serde_json::from_str::<Value>(actual).ok() == Some(expected_payload.clone())
        )
    }),
)
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        processor
            .blockchain
            .add_observer(pegin_event.confirmations.clone());
        let _ = processor.track_pegin_requested(pegin_flow_id, pegin_event);

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
        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let (block_1, block_2, _) = create_block_and_uncles();

        let pegin_requested = dummy_pegin_requested_event();
        let pegin_requested_event = PeginRequestedEvent {
            inner: pegin_requested.clone(),
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(10)),
        };

        let pegin_accepted = dummy_pegin_accepted_event();
        let pegin_accepted_event = PeginAcceptedEvent {
            inner: pegin_accepted,
            block_number: block_2.number(),
            block_hash: block_2.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(9)),
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
            .add_observer(pegin_accepted_event.confirmations.clone());

        let _ = processor.track_pegin_requested(pegin_flow_id, pegin_requested_event);
        let _ = processor.track_pegin_accepted(pegin_accepted_event);

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
                            IncomingBitVMXApiMessages::SetVar(_, name, VariableTypes::String(_))
                         if name == "PeginAccepted"
                    )
            })
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let pegin_requested_event = PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 99.into(),
            block_hash: BlockHash::from(H256::from_low_u64_be(122)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(10)),
        };

        let pegin_accepted = dummy_pegin_accepted_event();
        let pegin_accepted_event = PeginAcceptedEvent {
            inner: pegin_accepted,
            block_number: 100.into(),
            block_hash: BlockHash::from(H256::from_low_u64_be(123)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(11)),
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
            .add_observer(pegin_accepted_event.confirmations.clone());
        let _ = processor.track_pegin_requested(pegin_flow_id, pegin_requested_event);
        let _ = processor.track_pegin_accepted(pegin_accepted_event);

        let (block_1, _, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());

        assert_eq!(processor.tracker.len(), 0); // since pegin is completed at this point
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
