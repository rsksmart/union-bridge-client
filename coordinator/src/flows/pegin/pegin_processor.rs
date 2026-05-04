use std::any::type_name_of_val;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::Txid;
use common::msg_broker::bitvmx_types::{
    BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, PeginAcceptedMessage,
    RSK_PEGIN_TAG, TransactionStatus, VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::{BlockNumber, CommitteeId, Hash256, RskBlockAndUncles, TxIdParser};
use log::{debug, error, info, trace, warn};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use union_contracts::bindings::pegin_manager::PeginManager::PeginRequested;
use uuid::Uuid;

use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::event_processor::EventProcessor;
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
    BtcSignatureSubFlowFactoryApi,
};
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::flows::common::{GlobalContext, Signaling};
use crate::flows::pegin::pegin_flow::{PeginFlow, State, StepData, Steps};
use crate::flows::pegin::utils::get_temp_pegin_pid;
use crate::store::{
    CoordinatorStoreApi, StoreKey, StorePrefix, cleanup_flows_matching, restore_flows,
};
use crate::types::{
    AdminRequest, AllOperatorTakeTxidsAddedEvent, EventStatus, FlowKind, PeginAcceptedEvent,
    PeginRequestedEvent, RegisterSignaturesBitVmxData, RskPegManagerEvents, TickScheduler,
    UserRequests,
};

const PEGIN_ACCEPTED_INPUT_MSG: &str = "pegin_accepted";
const OPERATOR_TAKE_TX_PREFIX: &str = "OPERATOR_TAKE_TX";
const OPERATOR_WON_TX_PREFIX: &str = "OPERATOR_WON_TX";

fn is_missing_native_bridge_confirmations(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(domain_err) = cause.downcast_ref::<DomainErrors>() {
            matches!(domain_err, DomainErrors::MissingConfirmationsOnNativeBridge(_))
        } else {
            false
        }
    })
}

/// Processor that manages multiple pegin flow state machines
pub struct PeginFlowProcessor<CG, BC, BSF, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
    S: CoordinatorStoreApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    btc_sig_subflow_factory: FactoryBSF,
    pegin_flows: HashMap<Uuid, PeginFlow<CG, BC, S>>,
    signature_flows: HashMap<Uuid, BSF>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    tx_status_scheduler: TickScheduler<Uuid>,
    pegin_request_tracker: HashSet<Txid>,
    pending_pegin_requested: HashMap<Txid, PeginRequestedEvent>,
    // For retry logic when native bridge lacks confirmations
    unconfirmed_pegin_requests: HashMap<String, (BtcTxSPVProof, i16)>,
    pegin_retry_scheduler: TickScheduler<String>,
    unconfirmed_accept_pegin: HashMap<Uuid, i16>,
    accept_pegin_retry_scheduler: TickScheduler<Uuid>,
    store: Rc<S>,
    signaling: Rc<Signaling>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    required_confirmations: u32,
    btc_confirmations: u32,
    btc_status_retry_blocks: u32,
}

impl<CG, BC, S>
    PeginFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
        S,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        store: &Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        required_confirmations: u32,
        btc_confirmations: u32,
        btc_status_retry_blocks: u32,
    ) -> Self {
        let factory = BtcSignatureSubFlowFactory::new(
            Rc::clone(&contracts_gateway),
            rt_sync.clone(),
            required_confirmations,
        );

        // Subscribe to BitVMX pegin events
        Self::subscribe_to_bitvmx_pegin_events(&bitvmx_broker, btc_confirmations)
            .expect("Failed to subscribe to BitVMX pegin events");

        info!("Successfully subscribed to BitVMX pegin events");

        let mut processor = Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            btc_sig_subflow_factory: factory,
            pegin_flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            signature_flows: HashMap::new(),
            tx_status_scheduler: TickScheduler::new(),
            pegin_request_tracker: HashSet::new(),
            pending_pegin_requested: HashMap::new(),
            unconfirmed_pegin_requests: HashMap::new(),
            pegin_retry_scheduler: TickScheduler::new(),
            unconfirmed_accept_pegin: HashMap::new(),
            accept_pegin_retry_scheduler: TickScheduler::new(),
            store: Rc::clone(store),
            signaling,
            native_bridge_verifier,
            required_confirmations,
            btc_confirmations,
            btc_status_retry_blocks,
        };

        let flow_factory = |saved_state: State| {
            PeginFlow::from_saved_state(
                Rc::clone(&processor.contracts_gateway),
                processor.rt_sync.clone(),
                Rc::clone(&processor.bitvmx_broker),
                saved_state,
                Rc::clone(&processor.store),
                processor.signaling.clone(),
                processor.native_bridge_verifier.clone(),
            )
        };

        processor.pegin_flows = restore_flows(store.as_ref(), StorePrefix::PeginFlow, flow_factory)
            .expect("Failed to load flows from store");

        processor
    }

    /// Handle `PeginRequested` event by finding and updating existing flow.
    fn create_flow_for_pegin_requested(&mut self, event: &PeginRequested) -> Result<()> {
        let committee_id: CommitteeId = event.committeeId.into();

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PeginRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PeginRequested event with committee id {committee_id}, as member I should respond"
        );

        // Get the Bitcoin tx_id to find existing flow
        let btc_tx_id = TxIdParser::fb_32_to_txid(event.requestPeginTxid);
        let temp_flow_id = get_temp_pegin_pid(btc_tx_id);

        // Find the existing flow that should have been created from PeginTransactionFound
        if let Some(existing_flow) = self.pegin_flows.get_mut(&temp_flow_id) {
            info!(
                "Found existing pegin flow {temp_flow_id} for Bitcoin tx: {btc_tx_id}, completing PeginRequested step"
            );

            // Complete the step - this will trigger ID migration inside the flow and persist with new ID
            let step_data = StepData::PeginRequested(event.clone());
            existing_flow.complete_step(&step_data)?;

            // Get the new official flow ID after migration
            let official_flow_id = existing_flow.flow_id();

            // Move the flow to the new key in our map
            let flow = self
                .pegin_flows
                .remove(&temp_flow_id)
                .expect("Flow must exist as we just accessed it via get_mut");
            self.pegin_flows.insert(official_flow_id, flow);

            // Clean up the old temp entry from storage
            if let Err(e) = self.store.delete_flow(&StoreKey::PeginFlow(temp_flow_id)) {
                error!("Failed to delete temp flow state {temp_flow_id}: {e}");
            }

            info!(
                "Successfully migrated flow from temp ID {temp_flow_id} to official ID {official_flow_id}"
            );
        } else {
            warn!(
                "No existing temp flow found for Bitcoin tx: {btc_tx_id} (temp_id: {temp_flow_id}). This should not happen if PeginTransactionFound was processed."
            );
        }

        Ok(())
    }

    /// Handle confirmed `PeginAccepted` event
    fn handle_pegin_accepted(&mut self, pa: &PeginAcceptedEvent) -> Result<()> {
        info!("Processing confirmed PeginAccepted event: {pa:?}");

        // Find the flow corresponding to this pegin acceptance using accept_pegin_tx_hash
        let flow_opt = self.pegin_flows.values_mut().find(|flow| {
            flow.get_accept_pegin_txid().map(TxIdParser::txid_to_fb_32)
                == Some(pa.inner.acceptPeginTxid)
        });

        if let Some(flow) = flow_opt {
            let step_data = StepData::PeginAccepted(pa.inner.clone());
            flow.complete_step(&step_data)?;
        } else {
            bail!(
                "No matching pegin flow found for PeginAccepted event: acceptPeginTxid={}. This indicates a missing or corrupted flow state.",
                pa.inner.acceptPeginTxid
            );
        }
        Ok(())
    }

    /// Process confirmed RSK events
    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        info!("Processing confirmed RSK event: {event:?}");

        match event {
            RskPegManagerEvents::PeginRequested(pr) => {
                let committee_id = pr.inner.committeeId.into();
                if !self.global_context.my_committees().im_member(&committee_id) {
                    debug!(
                        "Handling PeginRequested event with committee id {committee_id}, I am NOT member so I skip"
                    );
                    return Ok(());
                }
                info!("Processing confirmed PeginRequested event: {pr:?}");
                let btc_tx_id = TxIdParser::fb_32_to_txid(pr.inner.requestPeginTxid);
                let temp_flow_id = get_temp_pegin_pid(btc_tx_id);
                let should_buffer = self
                    .pegin_flows
                    .get(&temp_flow_id)
                    .is_none_or(|flow| flow.current_step() != Steps::PeginRequested);

                if should_buffer {
                    info!(
                        "Buffering PeginRequested for Bitcoin tx: {btc_tx_id} until request SPV flow is ready"
                    );
                    self.pending_pegin_requested.insert(btc_tx_id, pr.clone());
                } else {
                    self.create_flow_for_pegin_requested(&pr.inner)?;
                }
            }
            RskPegManagerEvents::PeginAccepted(pa) => {
                self.handle_pegin_accepted(pa)?;
            }
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(aottah) => {
                self.handle_all_operator_take_tx_hashes_added(aottah)?;
            }
            _ => {
                trace!("Ignoring confirmed RSK event: {}", type_name_of_val(event));
            }
        }

        self.cleanup_terminal_flows();
        Ok(())
    }

    fn handle_all_operator_take_tx_hashes_added(
        &mut self,
        event: &AllOperatorTakeTxidsAddedEvent,
    ) -> Result<()> {
        debug!(
            "Processing AllOperatorTakeTxidsAdded: acceptPeginTxid={}",
            event.inner.acceptPeginTxid
        );

        // Find the flow by accept_pegin_tx_hash
        let flow_opt = self.pegin_flows.values_mut().find(|flow| {
            flow.get_accept_pegin_txid().map(TxIdParser::txid_to_fb_32)
                == Some(event.inner.acceptPeginTxid)
        });

        if let Some(flow) = flow_opt {
            let flow_id = flow.flow_id();

            // Start the BTC signature flow if not already started
            if self.signature_flows.contains_key(&flow_id) {
                error!("BTC signature flow already started: flow_id={flow_id}");
            } else {
                info!("Starting BTC signature flow: flow_id={flow_id}");

                let pegin_accepted = flow.get_bitvmx_pegin_accepted().ok_or_else(|| {
                    anyhow!("PeginAcceptedMessage not found for flow_id: {flow_id}.")
                })?;

                // Note: v0.2.0 contracts - initSignatures is called with acceptPeginTxid (the transaction ID),
                // not the signatureHash. So we must use acceptPeginTxid for addMemberNonce.
                let accept_pegin_txid = flow
                    .get_accept_pegin_txid()
                    .ok_or_else(|| anyhow!("acceptPeginTxid not found for flow_id: {flow_id}"))?;
                let hash_to_sign = Hash256::from(TxIdParser::txid_to_fb_32(accept_pegin_txid));
                let register_input = RegisterSignaturesBitVmxData {
                    hash_to_sign,
                    nonce: pegin_accepted.accept_pegin_nonce.clone(),
                    signature: pegin_accepted.accept_pegin_signature,
                };

                let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(flow_id);
                btc_sig_subflow.start_signature_flow(flow_id, &register_input)?;

                self.signature_flows.insert(flow_id, btc_sig_subflow);

                // Complete the step to move to the next state
                let step_data = StepData::OperatorTakeHashAdded;
                flow.complete_step(&step_data)?;
            }
        } else {
            debug!(
                "Received AllOperatorTakeTxidsAdded: unknown_acceptPeginTxid={:?}",
                event.inner.acceptPeginTxid
            );
        }

        Ok(())
    }

    /// Build event info for `PeginRequested` events
    fn build_pegin_requested_event_info(
        event: &PeginRequestedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegin-requested-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PeginRequested(event.clone()),
        )
    }

    fn build_pegin_accepted_event_info(
        event: &PeginAcceptedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegin-accepted-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PeginAccepted(event.clone()),
        )
    }

    fn build_all_operator_take_tx_hashes_added_event_info(
        event: &AllOperatorTakeTxidsAddedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("all-operator-take-tx-hashes-added-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(event.clone()),
        )
    }

    fn process_unhandled_confirmed_sig_flow_events(
        &mut self,
        block: &RskBlockAndUncles,
    ) -> Result<()> {
        let mut flows_to_dispatch = Vec::new();
        for (flow_id, signature_flow) in &mut self.signature_flows {
            signature_flow.delegate_block(block)?;
            if signature_flow.is_done() {
                flows_to_dispatch.push(*flow_id);
            }
        }

        for flow_id in &flows_to_dispatch {
            // Always remove the signature flow when it's done
            self.signature_flows.remove(flow_id);

            if let Some(flow) = self.pegin_flows.get_mut(flow_id) {
                // Only complete the step if the flow is still waiting for signatures
                if flow.current_step() != Steps::DispatchTransaction {
                    warn!(
                        "Signature flow completed for flow_id: {flow_id} but flow is at step {:?}, expected {:?}. Skipping dispatch step.",
                        flow.current_step(),
                        Steps::DispatchTransaction
                    );
                    continue;
                }

                let step_data = StepData::DispatchAcceptPeginTransaction;
                flow.complete_step(&step_data)?;
            } else {
                warn!(
                    "Signature flow done for unknown pegin flow_id: {flow_id}. Skipping dispatch step"
                );
            }
        }

        Ok(())
    }

    fn handle_transaction_status_received(
        &mut self,
        flow_id: &Uuid,
        tx_status: TransactionStatus,
    ) -> Result<()> {
        let Some(flow) = self.pegin_flows.get_mut(flow_id) else {
            trace!("Ignoring BitVMX Transaction event for unknown flow_id: {flow_id}");
            return Ok(());
        };

        let TransactionStatus { tx_id, confirmations, .. } = tx_status;
        let flow_id = flow.flow_id();
        let expected_txid = flow
            .get_accept_pegin_txid()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;

        if expected_txid != tx_id {
            return Err(anyhow!(
                "Pegin state for flow_id: {flow_id} does not match received tx_id: {tx_id:?} from tx status message"
            ));
        }

        if flow.current_step() != Steps::ConfirmAcceptPeginTransaction {
            return Err(anyhow!(
                "Mismatch current step for flow {} expected {:?} having {:?}",
                flow_id,
                Steps::ConfirmAcceptPeginTransaction,
                flow.current_step()
            ));
        }

        if confirmations >= self.btc_confirmations {
            debug!("Transaction confirmed with sufficient confirmations for flow_id: {flow_id}");
            let step_data = StepData::AcceptPeginTransactionConfirmed(tx_status);
            flow.complete_step(&step_data)?;
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
        } else {
            let min_conf = self.btc_confirmations;
            debug!(
                "Bitcoin transaction {tx_id} missing confirmations ({confirmations}/{min_conf}) for flow_id {flow_id}, rescheduling"
            );
            self.tx_status_scheduler.schedule(flow_id, self.btc_status_retry_blocks);
        }
        Ok(())
    }

    fn handle_transaction_status_tick(&mut self) -> Result<()> {
        if self.tx_status_scheduler.is_empty() {
            return Ok(());
        }

        let ready = self.tx_status_scheduler.tick();
        for flow_id in ready {
            match self.pegin_flows.get_mut(&flow_id) {
                Some(flow) => {
                    if flow.current_step() == Steps::ConfirmAcceptPeginTransaction {
                        flow.request_transaction_status()?;
                    } else {
                        warn!(
                            "Mismatch current step for flow {} expected {:?} having {:?}",
                            flow_id,
                            Steps::ConfirmAcceptPeginTransaction,
                            flow.current_step()
                        );
                    }
                }
                None => {
                    warn!("Skipping delayed transaction status request for unknown flow {flow_id}");
                }
            }
        }

        Ok(())
    }

    /// Resolve the temp and official ids that can point to the same pegin flow.
    /// If the relationship is not in memory, fall back to the supplied id.
    fn linked_pegin_flow_ids(&self, flow_id: Uuid) -> Vec<Uuid> {
        let mut ids = vec![flow_id];
        let flow = self.pegin_flows.get(&flow_id).or_else(|| {
            self.pegin_flows.values().find(|flow| {
                let ctx = &flow.get_state().ctx;
                ctx.temp_flow_id == Some(flow_id) || ctx.official_flow_id == Some(flow_id)
            })
        });

        if let Some(flow) = flow {
            let ctx = &flow.get_state().ctx;
            for id in [ctx.temp_flow_id, ctx.official_flow_id].into_iter().flatten() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// Mark a pegin flow as failed and stop pending local work for it.
    fn fail_flow(&mut self, flow_id: Uuid, reason: &str) -> Result<()> {
        let flow_ids_to_fail = self.linked_pegin_flow_ids(flow_id);

        let mut marked = false;
        for id in &flow_ids_to_fail {
            if let Some(flow) = self.pegin_flows.get_mut(id) {
                flow.mark_failed(reason)?;
                marked = true;
            }
        }

        if marked {
            warn!("Admin marked pegin flow {flow_id} as failed: {reason}");
        } else {
            warn!("Admin requested fail for unknown pegin flow {flow_id}: {reason}");
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn schedule_request_pegin_retry(
        &mut self,
        spv_proof: BtcTxSPVProof,
        attempt: i16,
        reason: &str,
    ) {
        let block_hash = spv_proof.block_hash.clone();
        info!("{reason} for block {block_hash} (attempt {attempt})");
        self.unconfirmed_pegin_requests.insert(block_hash.clone(), (spv_proof, attempt));
        self.pegin_retry_scheduler.schedule(block_hash, self.btc_status_retry_blocks);
    }

    fn schedule_accept_pegin_retry(&mut self, flow_id: Uuid, attempt: i16, reason: &str) {
        info!("{reason} for flow {flow_id} (attempt {attempt})");
        self.unconfirmed_accept_pegin.insert(flow_id, attempt);
        self.accept_pegin_retry_scheduler.schedule(flow_id, self.btc_status_retry_blocks);
    }

    fn handle_pegin_retry_tick(&mut self) -> Result<()> {
        if self.pegin_retry_scheduler.is_empty() && self.accept_pegin_retry_scheduler.is_empty() {
            return Ok(());
        }

        for block_hash in self.pegin_retry_scheduler.tick() {
            debug!("(Re)trying request_pegin for block {block_hash}");

            let Some((spv_proof, attempt)) = self.unconfirmed_pegin_requests.remove(&block_hash)
            else {
                warn!("No unconfirmed pegin request found for block: {block_hash}");
                continue;
            };

            let tx_id = spv_proof.tx.compute_txid();
            let Some(flow_id) = self.request_pegin_flow_id(&tx_id) else {
                warn!("No pegin flow found for request_pegin retry: tx_id={tx_id}");
                continue;
            };

            let Some(flow) = self.pegin_flows.get_mut(&flow_id) else {
                warn!("No pegin flow found for request_pegin retry: flow_id={flow_id}");
                continue;
            };

            let Err(err) = flow.complete_step(&StepData::RetryRequestPegin) else {
                info!("Request pegin succeeded on retry for block {block_hash}");
                self.pegin_request_tracker.remove(&tx_id);
                self.replay_pending_pegin_requested(&tx_id)?;
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for request_pegin: {err:?}");
                self.pegin_request_tracker.remove(&tx_id);
                continue;
            }

            let next_attempt = attempt.saturating_add(1);
            self.schedule_request_pegin_retry(
                spv_proof,
                next_attempt,
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }

        for flow_id in self.accept_pegin_retry_scheduler.tick() {
            let Some(attempt) = self.unconfirmed_accept_pegin.remove(&flow_id) else {
                warn!("No accept_pegin retry state found for flow {flow_id}");
                continue;
            };

            let Some(flow) = self.pegin_flows.get_mut(&flow_id) else {
                warn!("No pegin flow found for accept_pegin retry: {flow_id}");
                continue;
            };

            if flow.current_step() != Steps::AcceptPegin {
                debug!(
                    "Skipping accept_pegin retry for flow {flow_id} in step {:?}",
                    flow.current_step()
                );
                continue;
            }

            let Err(err) = flow.complete_step(&StepData::RetryAcceptPegin) else {
                info!("Accept pegin succeeded on retry for flow {flow_id}");
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for accept_pegin: {err:?}");
                continue;
            }

            let next_attempt = attempt.saturating_add(1);
            self.schedule_accept_pegin_retry(
                flow_id,
                next_attempt,
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }

        Ok(())
    }

    fn replay_pending_pegin_requested(&mut self, tx_id: &Txid) -> Result<()> {
        if let Some(pegin_requested_event) = self.pending_pegin_requested.remove(tx_id) {
            info!("Replaying buffered PeginRequested for Bitcoin tx: {tx_id}");
            self.process_confirmed_rsk_event(&RskPegManagerEvents::PeginRequested(
                pegin_requested_event,
            ))?;
        }

        Ok(())
    }

    fn stop_confirming_event(&mut self, id: &str) -> Option<ConfirmableEventWithData> {
        let mut event = self.events_confirming.remove(id)?;
        if let Err(e) = event.stop_confirming() {
            error!("Failed to stop confirming for event {id}: {e}");
        }
        if self.events_confirming.is_empty() {
            self.blockchain_view.clear();
        }
        Some(event)
    }

    fn process_block_confirmations(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            trace!("No events left to confirm, skipping block");
            return Ok(());
        }

        self.blockchain_view.update(block);

        // process confirmed events while removing them from the hashmap
        // collect the keys of confirmed events first to avoid mutating while iterating
        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter(|(_, event)| event.is_confirmed())
            .map(|(key, _)| key.clone())
            .collect();

        for key in confirmed_keys {
            if let Some(event) = self.stop_confirming_event(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn subscribe_to_bitvmx_pegin_events(bitvmx_broker: &BC, confirmations: u32) -> Result<()> {
        bitvmx_broker.send(IncomingBitVMXApiMessages::SubscribeToRskPegin(Some(confirmations)))?;
        Ok(())
    }

    fn handle_pegin_transaction_found(&mut self, tx_id: Txid) -> Result<()> {
        let temp_flow_id = get_temp_pegin_pid(tx_id);
        if self.pegin_request_tracker.contains(&tx_id)
            || self.pegin_flows.contains_key(&temp_flow_id)
        {
            debug!("Ignoring duplicate BitVMX pegin event for tx_id={tx_id}");
            return Ok(());
        }

        // Create a new pegin flow from Bitcoin transaction
        let mut flow = PeginFlow::new(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            tx_id,
            Rc::clone(&self.store),
            self.signaling.clone(),
            self.native_bridge_verifier.clone(),
        );

        info!("Created new pegin flow {temp_flow_id} from Bitcoin transaction: {tx_id}");

        // Advance the flow from PeginTransactionFound to RequestPeginSpvProof
        let step_data = StepData::PeginTransactionFound;
        flow.complete_step(&step_data)?;

        self.pegin_flows.insert(temp_flow_id, flow);
        self.pegin_request_tracker.insert(tx_id);

        Ok(())
    }

    fn request_pegin_flow_id(&self, tx_id: &Txid) -> Option<Uuid> {
        self.pegin_flows.iter().find_map(|(flow_id, flow)| {
            if flow.current_step() == Steps::RequestPeginSpvProof
                && flow.get_state().ctx.request_pegin_btc_tx_id == Some(*tx_id)
            {
                Some(*flow_id)
            } else {
                None
            }
        })
    }

    fn handle_spv_proof_for_request_pegin(
        &mut self,
        tx_id: &Txid,
        spv_proof: BtcTxSPVProof,
    ) -> Result<()> {
        if !self.pegin_request_tracker.contains(tx_id) {
            return Ok(()); // Not a tracked pegin request
        }

        info!("Handling request pegin SPV proof: tx_id={tx_id}");

        let Some(flow_id) = self.request_pegin_flow_id(tx_id) else {
            warn!("No pegin flow found for request_pegin: tx_id={tx_id}");
            return Ok(());
        };

        let Some(flow) = self.pegin_flows.get_mut(&flow_id) else {
            warn!("No pegin flow found for request_pegin: flow_id={flow_id}");
            return Ok(());
        };

        let step_data = StepData::RequestPeginSpvProof(spv_proof.clone());
        if let Err(err) = flow.complete_step(&step_data) {
            if is_missing_native_bridge_confirmations(&err) {
                self.schedule_request_pegin_retry(
                    spv_proof,
                    1,
                    "Missing confirmations on native bridge, scheduling retry",
                );
                return Ok(());
            }

            return Err(err);
        }

        self.pegin_request_tracker.remove(tx_id);
        debug!("Removed request_pegin_txid from tracking: tx_id={tx_id}");

        self.replay_pending_pegin_requested(tx_id)
    }

    fn handle_spv_proof_for_accept_pegin(
        &mut self,
        tx_id: &Txid,
        spv_proof: BtcTxSPVProof,
    ) -> Result<()> {
        // Find state by matching accept_pegin_txid from bitvmx_pegin_accepted
        let flow_opt =
            self.pegin_flows.values_mut().find(|flow| flow.get_accept_pegin_txid() == Some(*tx_id));

        if let Some(flow) = flow_opt {
            info!("Handling accept pegin SPV proof: flow_id={}, tx_id={}", flow.flow_id(), tx_id);
            let flow_id = flow.flow_id();
            let step_data = StepData::AcceptPeginSpvProof(spv_proof);
            if let Err(err) = flow.complete_step(&step_data) {
                if is_missing_native_bridge_confirmations(&err) {
                    let attempt = self
                        .unconfirmed_accept_pegin
                        .get(&flow_id)
                        .copied()
                        .unwrap_or(0)
                        .saturating_add(1);
                    self.schedule_accept_pegin_retry(
                        flow_id,
                        attempt,
                        "Missing confirmations on native bridge, scheduling retry",
                    );
                    return Ok(());
                }
                return Err(err);
            }
        }

        Ok(())
    }

    fn has_flow_waiting_for_accept_pegin_spv(&self, tx_id: &Txid) -> bool {
        self.pegin_flows.values().any(|flow| flow.get_accept_pegin_txid() == Some(*tx_id))
    }

    fn cleanup_terminal_flow_state(&mut self) {
        let terminal_flows: Vec<_> = self
            .pegin_flows
            .iter()
            .filter(|(_, flow)| flow.is_terminal())
            .map(|(map_flow_id, flow)| {
                let state = flow.get_state();
                let mut flow_ids = vec![*map_flow_id, flow.flow_id()];

                for flow_id in
                    [state.ctx.temp_flow_id, state.ctx.official_flow_id].into_iter().flatten()
                {
                    if !flow_ids.contains(&flow_id) {
                        flow_ids.push(flow_id);
                    }
                }

                (flow_ids, state.ctx.request_pegin_btc_tx_id)
            })
            .collect();

        for (flow_ids, request_pegin_btc_tx_id) in terminal_flows {
            for flow_id in flow_ids {
                self.signature_flows.remove(&flow_id);
                self.tx_status_scheduler.cancel(&flow_id);
                self.accept_pegin_retry_scheduler.cancel(&flow_id);
                self.unconfirmed_accept_pegin.remove(&flow_id);
            }

            if let Some(tx_id) = request_pegin_btc_tx_id {
                self.pegin_request_tracker.remove(&tx_id);
                self.pending_pegin_requested.remove(&tx_id);

                let retry_keys: Vec<_> = self
                    .unconfirmed_pegin_requests
                    .iter()
                    .filter(|(_, (spv_proof, _))| spv_proof.tx.compute_txid() == tx_id)
                    .map(|(key, _)| key.clone())
                    .collect();

                for key in retry_keys {
                    self.pegin_retry_scheduler.cancel(&key);
                    self.unconfirmed_pegin_requests.remove(&key);
                }
            }
        }
    }

    fn cleanup_terminal_flows(&mut self) {
        self.cleanup_terminal_flow_state();
        cleanup_flows_matching(
            self.store.as_ref(),
            StorePrefix::PeginFlow,
            &mut self.pegin_flows,
            PeginFlow::is_terminal,
        );
    }
}

impl<CG, BC, S> EventProcessor
    for PeginFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
        S,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        self.cleanup_terminal_flows();

        // Pegin flows are created from RSK / BTC events, not user requests, except for
        // the admin "fail flow" lever — handled here so cleanup runs alongside this
        // processor's in-memory state.
        if let UserRequests::Admin(AdminRequest::FailFlow { kind, flow_id, reason }) = req
            && *kind == FlowKind::Pegin
        {
            self.fail_flow(*flow_id, reason)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        self.cleanup_terminal_flows();

        trace!("Processing BitVMX event: {event:?}");

        match event {
            OutgoingBitVMXApiMessages::OutputPatternTransactionFound(tx_id, _tx_status, tag) => {
                if tag.as_slice() == RSK_PEGIN_TAG {
                    debug!(
                        "Received BitVMX OutputPatternTransactionFound for RSK pegin: tx_id={tx_id}"
                    );
                } else {
                    trace!(
                        "Ignoring BitVMX OutputPatternTransactionFound with non-pegin tag: {:?}",
                        String::from_utf8_lossy(tag)
                    );
                }
            }
            // Handle PeginTransactionFound from BitVMX
            OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, _tx_status) => {
                debug!("Received BitVMX PeginTransactionFound: tx_id={tx_id}");
                self.handle_pegin_transaction_found(*tx_id)?;
            }
            // Handle SPVProof from BitVMX (for both request and accept pegin)
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => {
                if let Some(spv_proof) = spv_proof_opt {
                    debug!("Received BitVMX SPVProof: tx_id={tx_id}");
                    trace!("Received spv_proof_data for tx_id={tx_id}: {spv_proof:?}");

                    // Route SPV proof to the appropriate handler based on context
                    if self.pegin_request_tracker.contains(tx_id) {
                        // Handle as request pegin SPV proof
                        self.handle_spv_proof_for_request_pegin(tx_id, spv_proof.clone())?;
                    } else if self.has_flow_waiting_for_accept_pegin_spv(tx_id) {
                        // Handle as accept pegin SPV proof
                        self.handle_spv_proof_for_accept_pegin(tx_id, spv_proof.clone())?;
                    } else {
                        debug!(
                            "SPV proof for tx_id: {tx_id} does not match any tracked pegin request or flow"
                        );
                    }
                } else {
                    return Err(anyhow!(
                        "Received BitVMX SPVProof event for tx_id: {tx_id}, but no SPV proof was included."
                    ));
                }
            }
            // Handle CommInfo from BitVMX
            OutgoingBitVMXApiMessages::CommInfo(req_id, comm_info) => {
                trace!("Received CommInfo from BitVMX req_id: {req_id}, comm_info: {comm_info:?}");
                // For any flow in GetCommInfo step, complete the step with the CommInfo
                for (flow_id, flow) in &mut self.pegin_flows {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Completing GetCommInfo step for flow {flow_id}");
                        let step_data = StepData::CommInfo(comm_info.clone());
                        flow.complete_step(&step_data)?;
                    }
                }
            }
            // Handle PeginAccepted variable from BitVMX
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), PEGIN_ACCEPTED_INPUT_MSG) =>
            {
                info!("Received PeginAccepted variable from BitVMX for flow_id: {flow_id}");
                debug!("PeginAccepted data: {data}");
                let pegin_accepted: PeginAcceptedMessage = serde_json::from_str(data)?;
                let flow = self
                    .pegin_flows
                    .get_mut(flow_id)
                    .ok_or_else(|| anyhow!("Flow not found for flow_id: {flow_id}"))?;

                if flow.current_step() != Steps::PreparePeginSetup {
                    return Err(anyhow!(
                        "Mismatch current step for flow {} expected {:?} having {:?}",
                        flow_id,
                        Steps::PreparePeginSetup,
                        flow.current_step()
                    ));
                }

                let step_data = StepData::BitvmxPeginAccepted(pegin_accepted);
                flow.complete_step(&step_data)?;
            }
            OutgoingBitVMXApiMessages::TransactionInfo(flow_id, tx_name, transaction) => {
                let txid = transaction.compute_txid();
                let Some(flow) = self.pegin_flows.get_mut(flow_id) else {
                    trace!("Ignoring BitVMX TransactionInfo for unknown flow_id: {flow_id}");
                    return Ok(());
                };

                debug!(
                    "Received BitVMX TransactionInfo for flow_id: {flow_id}, tx_name: {tx_name}, txid: {txid}"
                );

                match flow.current_step() {
                    Steps::RequestOperatorTakeTransactionInfo
                        if tx_name.starts_with(OPERATOR_TAKE_TX_PREFIX) =>
                    {
                        flow.complete_step(&StepData::TransactionInfo {
                            tx_name: tx_name.clone(),
                            txid,
                        })?;
                    }
                    Steps::RequestOperatorWonTransactionInfo
                        if tx_name.starts_with(OPERATOR_WON_TX_PREFIX) =>
                    {
                        flow.complete_step(&StepData::TransactionInfo {
                            tx_name: tx_name.clone(),
                            txid,
                        })?;
                    }
                    _ => {
                        trace!(
                            "Ignoring BitVMX TransactionInfo for flow {} in step {:?}",
                            flow_id,
                            flow.current_step()
                        );
                        return Ok(());
                    }
                }
            }
            // Handle SetupCompleted from BitVMX
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                if self.pegin_flows.contains_key(program_id) {
                    info!("Pegin setup was completed: flow_id={program_id}");
                } else {
                    trace!("Ignoring BitVMX SetupCompleted for unknown program_id: {program_id}");
                }
            }
            // Handle Transaction status from BitVMX
            OutgoingBitVMXApiMessages::Transaction(flow_id, tx_status, _tx_opt) => {
                self.handle_transaction_status_received(flow_id, tx_status.clone())?;
            }
            _ => {
                trace!("Ignoring BitVMX event: {event:?}");
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        self.cleanup_terminal_flows();

        match event {
            RskPegManagerEvents::AllNoncesReady(data)
            | RskPegManagerEvents::AllSignaturesReady(data) => {
                debug!("Handling signature event {data:?}");
                for (flow_id, sig_flow) in &mut self.signature_flows {
                    sig_flow.delegate_rsk_event(*flow_id, event)?;
                }
                return Ok(());
            }
            _ => {
                // Continue with the normal flow
            }
        }

        // useful for testing purposes
        if self.required_confirmations == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::PeginRequested(e) => Self::build_pegin_requested_event_info(e),
            RskPegManagerEvents::PeginAccepted(e) => Self::build_pegin_accepted_event_info(e),
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(e) => {
                Self::build_all_operator_take_tx_hashes_added_event_info(e)
            }
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if is_removal {
            warn!("Removing pending RSK event: {event:?}");

            if self.stop_confirming_event(&id).is_none() {
                warn!("Tried to remove non-existing pending event with id {id}");
            }
        } else {
            debug!("Adding new pending {event:?}, start confirming at block {block_num}");

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                self.required_confirmations,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event.start_confirming(block_num).context("Starting confirming")?;

            self.events_confirming.insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting Rootstock confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.cleanup_terminal_flows();

        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.handle_transaction_status_tick()?;
        self.handle_pegin_retry_tick()?;
        self.process_block_confirmations(block)?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PeginFlowProcessor");
        self.pegin_flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.tx_status_scheduler.clear();
        self.pegin_retry_scheduler.clear();
        self.unconfirmed_pegin_requests.clear();
        self.accept_pegin_retry_scheduler.clear();
        self.unconfirmed_accept_pegin.clear();
        self.pegin_request_tracker.clear();
        self.pending_pegin_requested.clear();
        self.signature_flows.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use alloy_primitives::{FixedBytes, I256};
    use bitcoin::Transaction;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
    use common::msg_broker::broker::MockBrokerClientApi;
    use primitive_types::H256;
    use union_contracts::bindings::pegin_manager::PeginManager::{
        RequestPeginTempInfo, StreamPosition,
    };

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::flows::pegin::pegin_flow::FlowContext;
    use crate::store::MockCoordinatorStoreApi;

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;

    type TestProcessor = PeginFlowProcessor<
        MockRskContractsGatewayApi,
        MockBitVmxBroker,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<MockRskContractsGatewayApi>>,
        BtcSignatureSubFlowFactory<MockRskContractsGatewayApi>,
        MockCoordinatorStoreApi,
    >;

    fn test_spv_proof() -> BtcTxSPVProof {
        BtcTxSPVProof {
            block_hash: "11".repeat(32),
            tx: Transaction {
                version: Version(2),
                lock_time: LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            merkle_branch_path: "0".to_string(),
            merkle_branch_hashes: vec![],
        }
    }

    fn test_pegin_requested_event(tx_id: Txid) -> PeginRequestedEvent {
        PeginRequestedEvent {
            inner: PeginRequested {
                committeeId: 1,
                requestPeginTxid: TxIdParser::txid_to_fb_32(tx_id),
                acceptPeginTxid: FixedBytes::<32>::ZERO,
                streamPosition: StreamPosition {
                    streamId: 0,
                    packetNumber: 0,
                    slotId: 0,
                    pegStatus: 0,
                },
                requestPeginInfo: RequestPeginTempInfo {
                    rskDestinationAddress: alloy_primitives::Address::ZERO,
                    btcReimbursementPubKey: FixedBytes::<32>::ZERO,
                    acceptPeginSignatureHash: FixedBytes::<32>::ZERO,
                    btcBlockNumber: I256::ZERO,
                    userReimbursementTxid: FixedBytes::<32>::ZERO,
                    rejectPeginTxid: FixedBytes::<32>::ZERO,
                },
                acceptPeginSignatureMessage: alloy_primitives::Bytes::new(),
            },
            block_number: BlockNumber::from(1),
            block_hash: common::types::BlockHash::from(H256::from_low_u64_be(2)),
            removed: false,
            tx_hash: common::types::TxHash::from(H256::from_low_u64_be(3)),
        }
    }

    fn test_processor(store: Rc<MockCoordinatorStoreApi>) -> TestProcessor {
        let contracts = Rc::new(MockRskContractsGatewayApi::new());
        let broker = Rc::new(MockBitVmxBroker::new());
        let rt_sync = RuntimeSync::new().expect("runtime");
        let factory = BtcSignatureSubFlowFactory::new(contracts.clone(), rt_sync.clone(), 1);

        PeginFlowProcessor {
            contracts_gateway: contracts,
            rt_sync,
            bitvmx_broker: broker,
            btc_sig_subflow_factory: factory,
            pegin_flows: HashMap::new(),
            signature_flows: HashMap::new(),
            global_context: GlobalContext::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            tx_status_scheduler: TickScheduler::new(),
            pegin_request_tracker: HashSet::new(),
            pending_pegin_requested: HashMap::new(),
            unconfirmed_pegin_requests: HashMap::new(),
            pegin_retry_scheduler: TickScheduler::new(),
            unconfirmed_accept_pegin: HashMap::new(),
            accept_pegin_retry_scheduler: TickScheduler::new(),
            store,
            signaling: Rc::new(Signaling::new("/tmp", "disabled")),
            native_bridge_verifier: NativeBridgeVerifier::Dummy,
            required_confirmations: 1,
            btc_confirmations: 1,
            btc_status_retry_blocks: 1,
        }
    }

    #[test]
    fn cleanup_terminal_flows_removes_pegin_flow_and_request_side_state() {
        let mut store = MockCoordinatorStoreApi::new();
        store.expect_delete_flow().returning(|_| Ok(()));
        let store = Rc::new(store);
        let mut processor = test_processor(store.clone());

        let spv_proof = test_spv_proof();
        let request_tx_id = spv_proof.tx.compute_txid();
        let temp_flow_id = get_temp_pegin_pid(request_tx_id);
        let official_flow_id = Uuid::new_v4();

        let state = State {
            flow_id: official_flow_id,
            ctx: FlowContext {
                flow_id: official_flow_id,
                step: Steps::Failed,
                temp_flow_id: Some(temp_flow_id),
                official_flow_id: Some(official_flow_id),
                request_pegin_btc_tx_id: Some(request_tx_id),
                request_pegin_btc_tx_status: None,
                request_pegin_spv_proof: None,
                pegin_requested: None,
                my_p2p_address: None,
                committee_output: None,
                bitvmx_pegin_accepted: None,
                operator_take_txid: None,
                operator_won_txid: None,
                accept_pegin_spv_proof: None,
                accept_pegin_tx_status: None,
                pegin_accepted: None,
                op_role: None,
            },
        };

        let flow = PeginFlow::from_saved_state(
            Rc::new(MockRskContractsGatewayApi::new()),
            RuntimeSync::new().expect("runtime"),
            Rc::new(MockBitVmxBroker::new()),
            state,
            store,
            Rc::new(Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        processor.pegin_flows.insert(temp_flow_id, flow);
        processor.tx_status_scheduler.schedule(official_flow_id, 1);
        processor.accept_pegin_retry_scheduler.schedule(official_flow_id, 1);
        processor.unconfirmed_accept_pegin.insert(official_flow_id, 1);
        processor.pegin_request_tracker.insert(request_tx_id);
        processor
            .pending_pegin_requested
            .insert(request_tx_id, test_pegin_requested_event(request_tx_id));
        processor
            .unconfirmed_pegin_requests
            .insert(spv_proof.block_hash.clone(), (spv_proof.clone(), 1));
        processor.pegin_retry_scheduler.schedule(spv_proof.block_hash.clone(), 1);

        processor.cleanup_terminal_flows();

        assert!(!processor.pegin_flows.contains_key(&temp_flow_id));
        assert!(!processor.tx_status_scheduler.is_scheduled(&official_flow_id));
        assert!(!processor.accept_pegin_retry_scheduler.is_scheduled(&official_flow_id));
        assert!(!processor.unconfirmed_accept_pegin.contains_key(&official_flow_id));
        assert!(!processor.pegin_request_tracker.contains(&request_tx_id));
        assert!(!processor.pending_pegin_requested.contains_key(&request_tx_id));
        assert!(!processor.unconfirmed_pegin_requests.contains_key(&spv_proof.block_hash));
        assert!(!processor.pegin_retry_scheduler.is_scheduled(&spv_proof.block_hash));
    }
}
