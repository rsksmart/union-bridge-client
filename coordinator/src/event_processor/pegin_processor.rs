use crate::{
    blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView},
    config::REQUIRED_CONFIRMATIONS,
    event_processor::EventProcessor,
    flows::btc_signature::btc_signature_subflow::{
        BtcSignatureSubFlowApi, BtcSignatureSubFlowFactoryApi,
    },
    types::{EventWithBlock, PeginAcceptedEvent, PeginRequestedEvent, RskPegManagerEvents},
};
use alloy_primitives::{FixedBytes, U256};
use anyhow::{Context, Result, bail};
use bitcoin::Txid;
use common::{
    msg_broker::{
        bitvmx_types::{
            BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, VariableTypes,
        },
        broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi},
    },
    runtime_sync::RuntimeSync,
    types::{RskBlockAndUncles, TxHash},
};
use log::info;
use serde::Serialize;
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, fmt::Debug, future::Future, rc::Rc};
use transaction_dispatcher::{
    rsk_gateway::{DomainErrors, RskContractsGatewayApi},
    types::{AcceptPeginInput, GetCommitteeInput, RequestPeginInput},
};
use union_contracts::bindings::{
    committee_registry::CommitteeRegistry::Committee,
    peg_manager::PegManager::{PeginAccepted, PeginRequested},
};
use uuid::Uuid;

const ACCEPT_PEGIN: &'static str = "accept-pegin";

#[derive(Debug, Clone, Serialize)]
struct PeginRequest {
    txid: FixedBytes<32>, // requestPeginTxHash
    amount: u64,
    accept_pegin_sighash: Vec<u8>, // acceptPeginSignatureMessage
    take_aggregated_key: FixedBytes<32>,
    operators_take_key: Vec<String>,
    slot_index: u32,
    committee_id: U256,
    rootstock_address: String,
    reimbursement_pubkey: FixedBytes<32>,
}

impl PeginRequest {
    fn build(event: PeginRequested, committee: Committee) -> Result<Self> {
        // collect only the member addresses that are operators
        let operators_take_key: Vec<String> = committee
            .members
            .iter()
            .filter(|m| m.role == 1) // assumes Role==1 is Operator
            .map(|m| m.memberAddress.to_checksum(None))
            .collect();

        // TODO(contracts-upgrade-alpha4): get real slot index from contract when available
        let slot_index: u32 = 0;

        let rootstock_address = event
            .requestPeginInfo
            .rskDestinationAddress
            .to_checksum(None);

        let accept_pegin_sighash = event.acceptPeginSignatureMessage.to_vec();

        Ok(Self {
            txid: event.requestPeginTxHash,
            amount: event.prevoutData.value,
            accept_pegin_sighash,
            take_aggregated_key: committee.aggregatedKey,
            operators_take_key,
            slot_index,
            committee_id: event.committeeId,
            rootstock_address,
            reimbursement_pubkey: event.requestPeginInfo.btcReimbursementPubKey,
        })
    }
}

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
struct PeginState<BSF: BtcSignatureSubFlowApi> {
    flow_id: Uuid,
    pegin_requested: PeginEvent<PeginRequested>,
    pegin_accepted: Option<PeginEvent<PeginAccepted>>,
    btc_signatures_flow: Option<BSF>,
}

impl<BSF: BtcSignatureSubFlowApi> PeginState<BSF> {
    fn new(pegin_flow_id: Uuid, pegin_requested: PeginEvent<PeginRequested>) -> Self {
        Self {
            flow_id: pegin_flow_id,
            pegin_requested,
            pegin_accepted: None,
            btc_signatures_flow: None,
        }
    }
}

pub struct PeginProcessor<CG, BC, BSF, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
{
    rt_sync: RuntimeSync,
    contracts: Rc<CG>,
    bitvmx_broker: Rc<BC>,
    blockchain: BlockchainView,
    tracker: HashMap<TxHash, PeginState<BSF>>,
    btc_sig_subflow_factory: FactoryBSF,
}

impl<CG, BC, BSF, FactoryBSF> PeginProcessor<CG, BC, BSF, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
{
    pub fn new(
        rt_sync: RuntimeSync,
        contracts: Rc<CG>,
        bitvmx_broker: Rc<BC>,
        factory: FactoryBSF,
    ) -> Self {
        Self::subscribe_to_bitvmx_pegin_events(&bitvmx_broker)
            .expect("Failed to subscribe to BitVMX pegin events");

        info!("Successfully subscribed to BitVMX pegin events");

        Self {
            rt_sync,
            contracts,
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            tracker: HashMap::new(),
            btc_sig_subflow_factory: factory,
        }
    }

    fn handle_pegin_requested(&mut self, data: &PeginRequestedEvent) -> Result<()> {
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

        self.track_pegin_requested(pegin_flow_id, pegin_requested)
    }

    fn handle_pegin_accepted(&mut self, data: &PeginAcceptedEvent) -> Result<()> {
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

        let observer_id = format!("pegin_accepted-{}", state.flow_id);
        let confirmations =
            BlockConfirmations::new(observer_id, data.block_number, REQUIRED_CONFIRMATIONS);
        let pegin_accepted = PeginEvent::new(data.clone(), confirmations);

        self.blockchain
            .add_observer(pegin_accepted.confirmations.clone());

        info!(
            "Adding PeginAccepted event to pegin event tracker. Event: {:?}",
            pegin_accepted
        );

        self.track_pegin_accepted(pegin_accepted)
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
            .insert(tx_hash, PeginState::new(pegin_flow_id, event));

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
                    tx_hash, state.flow_id
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
                    tx_hash, state.flow_id
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
            let flow_id = state.flow_id;

            let event = &mut state.pegin_requested;
            if !event.is_confirmed() || event.is_handled {
                continue;
            }

            let response = Self::call_contract(&self.rt_sync, "getCommittee", || async {
                self.contracts
                    .get_committee(GetCommitteeInput {
                        committee_id: event.data.inner.committeeId,
                    })
                    .await
            })?;

            let pegin_request = PeginRequest::build(event.data.inner.clone(), response.committee)?;

            Self::send_bitvmx_variable(
                &self.bitvmx_broker,
                flow_id,
                "PeginRequest",
                &pegin_request,
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
                "Successfully processed confirmed PeginRequested event for flow {}, starting BTC signature flow",
                flow_id
            );

            // now we start the signatures sub-flow
            state.btc_signatures_flow = Some(self.btc_sig_subflow_factory.create_flow(flow_id));
        }

        Ok(())
    }

    fn process_unhandled_confirmed_pegin_accepted_events(&mut self) -> Result<()> {
        let mut to_remove = Vec::new();

        for (tx_hash, state) in self.tracker.iter_mut() {
            let flow_id = state.flow_id;

            let event = match state.pegin_accepted.as_mut() {
                None => continue,
                Some(event) if !event.is_confirmed() || event.is_handled => continue,
                Some(event) => event,
            };

            Self::send_bitvmx_variable(
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

    fn subscribe_to_bitvmx_pegin_events(bitvmx_broker: &BC) -> Result<()> {
        // Used to subscribe to bitvmx pegin events, otherwise the client will not receive pegin
        // events from the bitvmx broker
        Self::send_to_bitvmx(
            bitvmx_broker,
            IncomingBitVMXApiMessages::SubscribeToRskPegin(),
        )
    }

    fn handle_pegin_transaction_found(&self, tx_id: Txid) -> Result<()> {
        // When notified of a new pegin tx found, the client will immediately
        // request the SPV proof of such transaction to notify the contract
        Self::send_to_bitvmx(
            &self.bitvmx_broker,
            IncomingBitVMXApiMessages::GetSPVProof(tx_id),
        )
    }

    fn send_bitvmx_variable<E: Serialize>(
        bitvmx_broker: &BC,
        pegin_flow_id: Uuid,
        variable_name: &str,
        data: &E,
    ) -> Result<()> {
        let data = serde_json::to_string(data)?;
        let message = IncomingBitVMXApiMessages::SetVar(
            pegin_flow_id,
            variable_name.to_string(),
            VariableTypes::String(data),
        );

        Self::send_to_bitvmx(bitvmx_broker, message)
    }

    fn send_to_bitvmx(bitvmx_broker: &BC, message: IncomingBitVMXApiMessages) -> Result<()> {
        bitvmx_broker.send(BROKER_SERVER_ID, message)?;

        Ok(())
    }

    fn handle_contract_invoke(&self, method_name: &str, json_data: &Value) -> Result<()> {
        match method_name {
            ACCEPT_PEGIN => {
                let spv_proof: BtcTxSPVProof = serde_json::from_value(json_data.clone())
                    .context("Failed to deserialize BtcTxSPVProof")?;
                let input: AcceptPeginInput = spv_proof.into();

                self.invoke_contract(ACCEPT_PEGIN, || async {
                    self.contracts.accept_pegin(input).await
                })
            }

            _ => bail!("Unsupported method: {}", method_name),
        }
    }

    fn handle_request_pegin(&self, spv_proof: BtcTxSPVProof) -> Result<()> {
        let input: RequestPeginInput = spv_proof.into();

        self.invoke_contract("requestPegin", || async {
            self.contracts.request_pegin(input).await
        })
    }

    fn invoke_contract<Fut, F, T>(&self, method_name: &str, invoke: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, DomainErrors>>,
        T: Debug,
    {
        info!(
            "Submitting contract transaction: method = '{}'",
            method_name
        );

        match self.rt_sync.run(invoke()) {
            Ok(_) => {
                info!("Successfully executed '{}'", method_name);
                Ok(())
            }
            Err(domain_err) => bail!("Error executing '{}': {:?}", method_name, domain_err),
        }
    }

    fn call_contract<Fut, F, T>(rt_sync: &RuntimeSync, method_name: &str, call: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, DomainErrors>>,
        T: Debug,
    {
        info!("Calling contract method: '{}'", method_name);

        match rt_sync.run(call()) {
            Ok(result) => {
                info!(
                    "Successfully called '{}', result: {:?}",
                    method_name, result
                );
                Ok(result)
            }
            Err(domain_err) => {
                bail!("Error calling '{}': {:?}", method_name, domain_err)
            }
        }
    }
}

impl<CG, BC, BSF, FactoryBSF> EventProcessor for PeginProcessor<CG, BC, BSF, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
{
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, _) => {
                info!(
                    "Received BitVMX PeginTransactionFound event with tx_id: {}",
                    tx_id
                );

                self.handle_pegin_transaction_found(tx_id.clone())?;
            }
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => match spv_proof_opt {
                Some(spv_proof) => {
                    info!(
                        "Received BitVMX SPVProof for tx_id: {}, proof: {:?}",
                        tx_id, spv_proof
                    );

                    self.handle_request_pegin(spv_proof.clone())?;
                }
                None => bail!(
                    "Received BitVMX SPVProof event for tx_id: {}, but no SPV proof was included.",
                    tx_id
                ),
            },
            OutgoingBitVMXApiMessages::Variable(
                pegin_flow_id,
                method,
                VariableTypes::String(data),
            ) if matches!(method.as_str(), ACCEPT_PEGIN) => {
                info!(
                    "Received BitVMX Variable Event. Flow Id: {}, Method: {}, Payload: {:?}",
                    pegin_flow_id, method, data
                );

                let json_data: Value = serde_json::from_str(data)?;

                self.handle_contract_invoke(method, &json_data)?;
            }
            // TODO(signatures-2) delegate SIGNATURE_MESSAGE message to BtcSignatureFlow::process_new_bitvmx_event, it is the response to request-pegin event we send them
            //  it looks like for now they do not include hash_to_sign in the message (see TODO in BitVmxSigningInfo), so we need to inject it in the OutgoingBitVMXApiMessages from the calling flow
            _ => {}
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::PeginRequested(data) => self.handle_pegin_requested(data),
            RskPegManagerEvents::PeginAccepted(data) => self.handle_pegin_accepted(data),
            // TODO(signatures-3) delegate AllNoncesReady and AllSignaturesReady to BtcSignatureFlow::process_new_rsk_event
            _ => Ok(()),
        }
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.tracker.is_empty() {
            return Ok(());
        }

        self.blockchain.update(block.clone());

        self.process_unhandled_confirmed_pegin_requested_events()?;
        self.process_unhandled_confirmed_pegin_accepted_events()?;

        // TODO(signatures-4) delegate to BtcSignatureFlow::process_new_block

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
    use crate::flows::btc_signature::btc_signature_subflow::{
        MockBtcSigSubFlowFactory, MockBtcSignatureSubFlowApi,
    };
    use crate::{
        coordinator::tests::MockRskContractsGatewayApi,
        event_processor::EventProcessor,
        types::{PeginAcceptedEvent, PeginRequestedEvent},
    };
    use alloy_primitives::{Address, Bytes, FixedBytes, U256, address};
    use anyhow::anyhow;
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
        hashes::Hash, transaction::Version,
    };
    use common::{
        msg_broker::{
            bitvmx_types::{TransactionBlockchainStatus, TransactionStatus},
            broker::{BROKER_SERVER_ID, BrokerError, MockBrokerClientApi},
        },
        test_utils::rsk_block_generator::create_block_and_uncles,
        types::BlockHash,
    };
    use hex::FromHex;
    use mockall::predicate::{eq, function};
    use primitive_types::H256;
    use serde_json::json;
    use transaction_dispatcher::types::GetCommitteeOutput;
    use transaction_dispatcher::{
        rsk_gateway::DomainErrors,
        types::{AcceptPeginOutput, RequestPeginOutput},
    };
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{CommitteeMember, Role};
    use union_contracts::bindings::peg_manager::PegManager::{
        PeginRequested, PrevoutData, RequestPeginTempInfo, StreamPosition,
    };

    #[test]
    fn subscribe_to_bitvmx_pegin_events_succeeds() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        // No need to test the returned processor here,
        // just that subscription succeeds and panics if not
        PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );
    }

    #[test]
    #[should_panic(expected = "Failed to subscribe to BitVMX pegin events")]
    fn subscribe_to_bitvmx_pegin_events_fails_and_panics() {
        let mut broker = MockBrokerClientApi::new();
        broker
            .expect_send()
            .times(1)
            .withf(|id, msg| {
                *id == BROKER_SERVER_ID
                    && matches!(msg, IncomingBitVMXApiMessages::SubscribeToRskPegin())
            })
            .returning(|_, _| {
                Err(BrokerError::UnknownError(anyhow!(
                    "simulated broker failure"
                )))
            });

        // Should panic when trying to subscribe
        PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );
    }

    #[test]
    fn process_new_bitvmx_pegin_transaction_found_should_send_get_svp_proof_bitvmx_message() {
        let status = dummy_transaction_status();
        let txid = status.tx_id;

        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        broker
            .expect_send()
            .withf(move |to, msg| {
                *to == BROKER_SERVER_ID
                    && matches!(msg, IncomingBitVMXApiMessages::GetSPVProof(id) if id == &txid)
            })
            .times(1)
            .returning(|_, _| Ok(true));

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let event = OutgoingBitVMXApiMessages::PeginTransactionFound(txid, status);
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_empty_spv_proof_event_should_return_error() {
        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let tx_id = dummy_spv_proof().tx.compute_txid();
        let event = OutgoingBitVMXApiMessages::SPVProof(tx_id, None);

        // Run and assert
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn process_new_bitvmx_spv_proof_event_for_request_pegin_should_call_request_pegin() {
        // Prepare the mocked contracts gateway
        let mut contracts = MockRskContractsGatewayApi::new();
        let expected_receipt = RequestPeginOutput {
            transaction_hash: "0x4e3f8a2d39c1b872b77e8a5c9a24be8f1d489ea7cf2d38375f18b5b54e7df662"
                .to_string(),
            success: true,
        };
        contracts
            .expect_request_pegin()
            .times(1)
            .returning(move |_| Ok(expected_receipt.clone()));

        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(
            rt_sync,
            contracts.into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let spv_proof = dummy_spv_proof();
        let tx_id = spv_proof.tx.compute_txid();
        let event = OutgoingBitVMXApiMessages::SPVProof(tx_id, Some(spv_proof));

        // Run and assert
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_spv_proof_event_for_request_pegin_should_fail_on_dispatch_error() {
        // Prepare a mocked contracts gateway that simulates a failure
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts
            .expect_request_pegin()
            .times(1)
            .returning(|_| Err(DomainErrors::UnknownContractError("simulated error".into())));

        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(
            rt_sync,
            contracts.into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let spv_proof = dummy_spv_proof();
        let tx_id = spv_proof.tx.compute_txid();
        let event = OutgoingBitVMXApiMessages::SPVProof(tx_id, Some(spv_proof));

        // We expect an error due to contract dispatch failure
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn process_new_bitvmx_pegin_accepted_event_does_not_send_response() {
        // Prepare the mocked contracts gateway
        let mut contracts = MockRskContractsGatewayApi::new();
        let expected_receipt = AcceptPeginOutput {
            transaction_hash: "0x7e8f27d21c8a0cfebfd2c647db4687e51eae3eaecdbf9f247c9057be682176a3"
                .to_string(),
            success: true,
        };
        contracts
            .expect_accept_pegin()
            .times(1)
            .returning(move |_| Ok(expected_receipt.clone()));

        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(
            rt_sync,
            contracts.into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        // Simulate event payload
        let spv_proof = dummy_spv_proof();
        let payload = serde_json::to_string(&spv_proof).unwrap();
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
            .expect_accept_pegin()
            .times(1)
            .returning(|_| Err(DomainErrors::UnknownContractError("simulated error".into())));

        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        // Runtime and processor initialization
        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(
            rt_sync,
            contracts.into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        // Payload
        let spv_proof = dummy_spv_proof();
        let payload = serde_json::to_string(&spv_proof).unwrap();
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
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
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

        let result = processor.process_new_rsk_event(&event);
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
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
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

        let result = processor.process_new_rsk_event(&event);
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

        let result = processor.process_new_rsk_event(&event);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 0);
        assert!(!processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_new_event_pegin_accepted_event_and_observer() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 122.into(),
            block_hash: BlockHash::from(H256::from([0xba; 32])),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });
        let result = processor.process_new_rsk_event(&event);
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

        let result = processor.process_new_rsk_event(&event);
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
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let pegin_requested = dummy_pegin_requested_event();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested,
            block_number: 122.into(),
            block_hash: BlockHash::from(H256::from([0xba; 32])),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(9)),
        });
        let result = processor.process_new_rsk_event(&event);
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

        let result = processor.process_new_rsk_event(&event);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 1);

        let event = RskPegManagerEvents::PeginAccepted(PeginAcceptedEvent {
            inner: dummy_pegin_accepted_event(),
            block_number: 456.into(),
            block_hash: BlockHash::from(H256::from([0xbb; 32])),
            removed: true, // event is removed
            tx_hash: TxHash::from(H256::from_low_u64_be(10)),
        });

        let result = processor.process_new_rsk_event(&event);
        let observer_id = processor.tracker.get(&tx_hash).unwrap().flow_id.to_string();
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
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let result = processor.process_new_rsk_event(&RskPegManagerEvents::UnknownEvent);
        assert!(result.is_ok());
        assert_eq!(processor.tracker.len(), 0);
    }

    #[test]
    fn process_new_block_ignores_if_no_pending_events() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
        );

        let (block_1, _, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        let result = processor.process_new_block(&block);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_block_adds_confirmations_for_register_pegin_but_event_not_confirmed() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
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

        let mut contracts = MockRskContractsGatewayApi::new();
        let committee = dummy_committee();
        let committee_clone_for_build = committee.clone();

        contracts
            .expect_get_committee()
            .withf(move |inp: &GetCommitteeInput| inp.committee_id == pegin_requested.committeeId)
            .returning(move |_| {
                Ok(GetCommitteeOutput {
                    committee: committee.clone(),
                })
            })
            .times(1);

        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let expected_pegin_request =
            PeginRequest::build(pegin_requested.clone(), committee_clone_for_build).unwrap();
        let expected_payload = json!(expected_pegin_request);

        broker
        .expect_send()
        .times(1)
        .with(
            eq(BROKER_SERVER_ID),
            function(move |req: &IncomingBitVMXApiMessages| {
                matches!(
                    req,
                    IncomingBitVMXApiMessages::SetVar(_, var_name, VariableTypes::String(actual))
                        if var_name == "PeginRequest"
                        && serde_json::from_str::<Value>(actual).ok() == Some(expected_payload.clone())
                )
            }),
        )
        .returning(|_, _| Ok(true));

        let mut mock_btc_sig_subflow_factory = MockBtcSigSubFlowFactory::new();
        mock_btc_sig_subflow_factory
            .expect_create_flow()
            .times(1)
            .returning(move |_| MockBtcSignatureSubFlowApi::new());

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            contracts.into(),
            broker.into(),
            mock_btc_sig_subflow_factory,
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
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
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
        expect_bitvmx_subscription_success(&mut broker);
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
            MockBtcSigSubFlowFactory::new(),
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

    fn expect_bitvmx_subscription_success(
        broker: &mut MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
    ) {
        broker
            .expect_send()
            .times(1)
            .withf(|id, msg| {
                *id == BROKER_SERVER_ID
                    && matches!(msg, IncomingBitVMXApiMessages::SubscribeToRskPegin())
            })
            .returning(|_, _| Ok(true));
    }

    fn dummy_transaction_status() -> TransactionStatus {
        let tx = Transaction {
            version: Version::ONE,
            lock_time: LockTime::from_height(0).unwrap(),
            input: vec![],
            output: vec![],
        };

        let txid = tx.compute_txid();

        TransactionStatus {
            tx_id: txid,
            tx,
            block_info: None,
            confirmations: 0,
            status: TransactionBlockchainStatus::Confirmed,
        }
    }

    fn dummy_spv_proof() -> BtcTxSPVProof {
        let tx = Transaction {
            version: Version::ONE,
            lock_time: LockTime::from_consensus(0),
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: Txid::from_byte_array(
                        <[u8; 32]>::from_hex(
                            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        )
                        .unwrap(),
                    ),
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence(429496729),
                witness: bitcoin::Witness::default(),
            }],
            output: vec![TxOut {
                value: Amount::from_btc(1.0).unwrap(),
                script_pubkey: ScriptBuf::new(),
            }],
        };

        BtcTxSPVProof {
            block_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            tx,
            merkle_branch_path: "left-right-left".to_string(),
            merkle_branch_hashes: vec![
                <[u8; 32]>::from_hex(
                    "1111111111111111111111111111111111111111111111111111111111111111",
                )
                .unwrap(),
                <[u8; 32]>::from_hex(
                    "2222222222222222222222222222222222222222222222222222222222222222",
                )
                .unwrap(),
            ],
        }
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

    fn dummy_committee() -> Committee {
        let leader: Address = address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
        Committee {
            aggregatedKey: FixedBytes::from([0x11u8; 32]),
            members: vec![
                CommitteeMember {
                    memberAddress: leader,
                    role: Role::from(1u8).into(), // Operator
                },
                CommitteeMember {
                    memberAddress: address!("0x0000000000000000000000000000000000000001"),
                    role: Role::from(2u8).into(), // Non-operator, should be filtered out
                },
            ],
            leaderAddress: leader,
            operatorTakeIndex: U256::from(0u64),
        }
    }
}
