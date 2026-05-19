use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, ensure};
use bitcoin::Txid;
use common::msg_broker::bitvmx_types::{
    BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, REJECT_PEGIN_TX,
    TransactionStatus,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::RskBlockAndUncles;
use tracing::{debug, error, info, trace, warn};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use uuid::Uuid;

use super::{RejectPeginFlow, RejectPeginTrigger, StepData, Steps};
use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::event_processor::EventProcessor;
use crate::flows::common::GlobalContext;
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::store::{CoordinatorStoreApi, StorePrefix, cleanup_flows_matching, restore_flows};
use crate::types::{
    EventStatus, RejectPeginRegisteredEvent, RskPegManagerEvents, TickScheduler, UserRequests,
};

fn is_missing_native_bridge_confirmations(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(domain_err) = cause.downcast_ref::<DomainErrors>() {
            matches!(domain_err, DomainErrors::MissingConfirmationsOnNativeBridge(_))
        } else {
            false
        }
    })
}

#[derive(Clone, Debug)]
pub(crate) struct RejectPeginProcessorConfig {
    /// Minimum BTC transaction confirmations for reject pegin (default: 1)
    pub min_tx_confirmations: u32,
    /// Blocks delay before rechecking transaction status (default: 20)
    pub blocks_delay_for_tx_check: u32,
    /// Required RSK block confirmations for `RejectPeginRegistered` (default: 5)
    pub required_confirmations: u32,
}

impl Default for RejectPeginProcessorConfig {
    fn default() -> Self {
        Self { min_tx_confirmations: 1, blocks_delay_for_tx_check: 20, required_confirmations: 5 }
    }
}

pub(crate) struct RejectPeginProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    bitvmx_broker: Rc<BC>,
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    global_context: GlobalContext,
    config: RejectPeginProcessorConfig,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    flows: HashMap<Uuid, RejectPeginFlow<CG, BC, S>>,
    tx_status_scheduler: TickScheduler<Uuid>,
    unconfirmed_register_reject_pegin: HashMap<Uuid, i16>,
    register_reject_pegin_retry_scheduler: TickScheduler<Uuid>,
    store: Rc<S>,
}

impl<CG, BC, S> RejectPeginProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        bitvmx_broker: Rc<BC>,
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        global_context: GlobalContext,
        store: &Rc<S>,
        config: RejectPeginProcessorConfig,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        let mut processor = Self {
            bitvmx_broker,
            contracts_gateway,
            rt_sync,
            global_context,
            config,
            native_bridge_verifier,
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            flows: HashMap::new(),
            tx_status_scheduler: TickScheduler::new(),
            unconfirmed_register_reject_pegin: HashMap::new(),
            register_reject_pegin_retry_scheduler: TickScheduler::new(),
            store: Rc::clone(store),
        };

        let flow_factory = |saved_state| {
            RejectPeginFlow::from_saved_state(
                Rc::clone(&processor.contracts_gateway),
                processor.rt_sync.clone(),
                Rc::clone(&processor.bitvmx_broker),
                saved_state,
                Rc::clone(&processor.store),
                processor.native_bridge_verifier.clone(),
            )
        };

        processor.flows = restore_flows(store.as_ref(), StorePrefix::RejectPeginFlow, flow_factory)
            .expect("Failed to load reject pegin flows from store");
        processor.rehydrate_pending_tx_status_checks();

        processor
    }

    fn rehydrate_pending_tx_status_checks(&mut self) {
        let pending_checks: Vec<Uuid> = self
            .flows
            .iter()
            .filter_map(|(flow_id, flow)| {
                if flow.current_step() == Steps::GetRejectTxConfirmation {
                    if flow.get_reject_pegin_txid().is_some() {
                        return Some(*flow_id);
                    }

                    warn!(
                        "RejectPeginProcessor restored flow {flow_id} in GetRejectTxConfirmation without a tracked tx id"
                    );
                }

                None
            })
            .collect();

        for flow_id in pending_checks {
            debug!("RejectPeginProcessor rehydrating tx status polling for flow {flow_id}");
            self.tx_status_scheduler.schedule(flow_id, 0);
        }
    }

    fn start_reject_pegin_flow(&mut self, trigger: RejectPeginTrigger) -> Result<()> {
        ensure!(
            self.global_context.my_committees().im_member(&trigger.committee_id),
            "Reject pegin requested for committee {} but this member is not part of it",
            trigger.committee_id
        );

        let flow = RejectPeginFlow::new(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            trigger,
            Rc::clone(&self.store),
            self.native_bridge_verifier.clone(),
        );
        let protocol_id = flow.protocol_id();
        flow.start()?;
        self.flows.insert(protocol_id, flow);
        Ok(())
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    fn handle_reject_pegin_tx_received(
        &mut self,
        program_id: Uuid,
        tx_status: &TransactionStatus,
    ) -> Result<()> {
        let Some(flow) = self.flows.get_mut(&program_id) else {
            trace!(
                "RejectPeginProcessor ignoring REJECT_PEGIN_TX for unknown program {program_id}"
            );
            return Ok(());
        };

        if flow.current_step() != Steps::GetRejectTxConfirmation {
            trace!(
                "RejectPeginProcessor ignoring REJECT_PEGIN_TX for program {} in step {:?}",
                program_id,
                flow.current_step()
            );
            return Ok(());
        }

        flow.record_reject_pegin_tx_status(tx_status)?;

        let confirmations = tx_status.confirmations;
        let min_conf = self.config.min_tx_confirmations;
        if confirmations >= min_conf {
            info!(
                "Reject pegin tx {} has enough confirmations ({}/{min_conf}), requesting SPV proof",
                tx_status.tx_id, confirmations
            );
            flow.complete_step(StepData::RejectPeginTxConfirmed(tx_status.clone()))?;
        } else {
            debug!(
                "Reject pegin tx {} needs more confirmations ({}/{}), scheduling status recheck",
                tx_status.tx_id, confirmations, min_conf
            );
            self.tx_status_scheduler.schedule(program_id, self.config.blocks_delay_for_tx_check);
        }
        Ok(())
    }

    fn handle_transaction_status_tick(&mut self) -> Result<()> {
        let ready = self.tx_status_scheduler.tick();
        for program_id in ready {
            let Some(flow) = self.flows.get(&program_id) else {
                warn!(
                    "RejectPeginProcessor: skipping tx status request for unknown flow {program_id}"
                );
                continue;
            };
            if flow.current_step() != Steps::GetRejectTxConfirmation {
                debug!(
                    "RejectPeginProcessor: skipping tx status request for flow {} in step {:?}",
                    program_id,
                    flow.current_step()
                );
                continue;
            }
            let msg = match flow.request_transaction_status() {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        "RejectPeginProcessor: flow {program_id} request_transaction_status: {e}"
                    );
                    continue;
                }
            };
            self.send_bitvmx_msg(msg)?;
        }
        Ok(())
    }

    fn schedule_register_reject_pegin_retry(&mut self, flow_id: Uuid, attempt: i16, reason: &str) {
        info!("{reason} for flow {flow_id} (attempt {attempt})");
        self.unconfirmed_register_reject_pegin.insert(flow_id, attempt);
        self.register_reject_pegin_retry_scheduler
            .schedule(flow_id, self.config.blocks_delay_for_tx_check);
    }

    fn handle_register_reject_pegin_retry_tick(&mut self) {
        if self.register_reject_pegin_retry_scheduler.is_empty() {
            return;
        }

        for flow_id in self.register_reject_pegin_retry_scheduler.tick() {
            let Some(attempt) = self.unconfirmed_register_reject_pegin.remove(&flow_id) else {
                warn!("No register_reject_pegin retry state found for flow {flow_id}");
                continue;
            };

            let retry_result = {
                let Some(flow) = self.flows.get_mut(&flow_id) else {
                    warn!("No reject pegin flow found for register_reject_pegin retry: {flow_id}");
                    continue;
                };

                if flow.current_step() != Steps::RegisterRejectPeginSpv {
                    debug!(
                        "Skipping register_reject_pegin retry for flow {flow_id} in step {:?}",
                        flow.current_step()
                    );
                    continue;
                }

                flow.complete_step(StepData::RetryRegisterRejectPegin)
            };

            match retry_result {
                Ok(()) => {
                    info!("Register reject pegin succeeded on retry for flow {flow_id}");
                    cleanup_flows_matching(
                        self.store.as_ref(),
                        StorePrefix::RejectPeginFlow,
                        &mut self.flows,
                        RejectPeginFlow::is_done,
                    );
                }
                Err(err) if is_missing_native_bridge_confirmations(&err) => {
                    self.schedule_register_reject_pegin_retry(
                        flow_id,
                        attempt.saturating_add(1),
                        "Still missing confirmations on native bridge, scheduling another retry",
                    );
                }
                Err(err) => {
                    error!("Error on retry for register_reject_pegin: {err:?}");
                }
            }
        }
    }

    fn handle_spv_proof(&mut self, tx_id: Txid, spv_proof: BtcTxSPVProof) -> Result<()> {
        let protocol_id = self
            .flows
            .iter()
            .find(|(_, f)| f.get_reject_pegin_txid() == Some(tx_id))
            .map(|(id, _)| *id);
        let Some(protocol_id) = protocol_id else {
            trace!("RejectPeginProcessor ignoring SPVProof for unknown tx_id {tx_id}");
            return Ok(());
        };
        let Some(flow) = self.flows.get_mut(&protocol_id) else {
            trace!("RejectPeginProcessor ignoring SPVProof for missing flow {protocol_id}");
            return Ok(());
        };
        if flow.current_step() != Steps::GetRejectPeginSpvProof {
            trace!(
                "RejectPeginProcessor ignoring SPVProof for flow {} in step {:?}",
                protocol_id,
                flow.current_step()
            );
            return Ok(());
        }
        if let Err(err) = flow.complete_step(StepData::RejectPeginSpvProof(spv_proof)) {
            if is_missing_native_bridge_confirmations(&err) {
                let attempt = self
                    .unconfirmed_register_reject_pegin
                    .get(&protocol_id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
                self.schedule_register_reject_pegin_retry(
                    protocol_id,
                    attempt,
                    "Missing confirmations on native bridge, scheduling retry",
                );
                return Ok(());
            }
            return Err(err);
        }
        cleanup_flows_matching(
            self.store.as_ref(),
            StorePrefix::RejectPeginFlow,
            &mut self.flows,
            RejectPeginFlow::is_done,
        );
        Ok(())
    }

    fn handle_reject_pegin_registered(&mut self, event: &RejectPeginRegisteredEvent) -> Result<()> {
        let reject_pegin_txid = event.inner.reject_pegin_txid;
        let request_pegin_txid = event.inner.request_pegin_txid;

        let Some(flow) = self.flows.values_mut().find(|flow| {
            flow.get_reject_pegin_txid() == Some(reject_pegin_txid)
                && flow.trigger().request_pegin_txid == request_pegin_txid
        }) else {
            trace!(
                "RejectPeginProcessor ignoring RejectPeginRegistered for reject_pegin_txid {reject_pegin_txid} request_pegin_txid {request_pegin_txid} - no matching flow",
            );
            return Ok(());
        };

        if flow.current_step() != Steps::RegisterRejectPeginSpv {
            trace!(
                "RejectPeginProcessor ignoring RejectPeginRegistered for flow {} in step {:?}",
                flow.protocol_id(),
                flow.current_step()
            );
            return Ok(());
        }

        info!(
            "RejectPeginRegistered confirmed for request_pegin_txid {request_pegin_txid} reject_pegin_txid {reject_pegin_txid}"
        );
        flow.complete_step(StepData::RejectPeginRegistered(event.inner.clone()))?;
        Ok(())
    }

    fn has_flow_for_reject_pegin_registered(&self, event: &RejectPeginRegisteredEvent) -> bool {
        let reject_pegin_txid = event.inner.reject_pegin_txid;
        let request_pegin_txid = event.inner.request_pegin_txid;

        self.flows.values().any(|flow| {
            flow.get_reject_pegin_txid() == Some(reject_pegin_txid)
                && flow.trigger().request_pegin_txid == request_pegin_txid
        })
    }

    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RejectPeginRegistered(reject_pegin_registered) => {
                self.handle_reject_pegin_registered(reject_pegin_registered)?;
            }
            _ => {
                trace!("RejectPeginProcessor ignoring confirmed RSK event {event:?}");
            }
        }

        cleanup_flows_matching(
            self.store.as_ref(),
            StorePrefix::RejectPeginFlow,
            &mut self.flows,
            RejectPeginFlow::is_done,
        );
        Ok(())
    }

    fn build_reject_pegin_registered_event_info(
        event: &RejectPeginRegisteredEvent,
    ) -> (String, EventStatus, common::types::BlockNumber, RskPegManagerEvents) {
        (
            format!("reject-pegin-registered-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::RejectPeginRegistered(event.clone()),
        )
    }

    fn process_block_confirmations(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            return Ok(());
        }

        self.blockchain_view.update(block);

        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter(|(_, event)| event.is_confirmed())
            .map(|(key, _)| key.clone())
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("Reject pegin RSK event confirmed, removing pending {key}");
                if let Err(err) = event.stop_confirming() {
                    warn!("Failed to stop confirming reject pegin event {key}: {err}");
                }
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn active_flows_len(&self) -> usize {
        self.flows.len()
    }

    #[cfg(test)]
    pub(crate) fn active_flow_ids(&self) -> Vec<Uuid> {
        self.flows.keys().copied().collect()
    }
}

impl<CG, BC, S> EventProcessor for RejectPeginProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        match req {
            UserRequests::RejectPegin(input) => {
                self.start_reject_pegin_flow(input.clone().into())?;
            }
            _ => {
                trace!("RejectPeginProcessor ignoring user request {req:?}");
            }
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::CommInfo(req_id, comm_info) => {
                trace!("RejectPeginProcessor received CommInfo for {req_id}: {comm_info:?}");
                for (flow_id, flow) in &mut self.flows {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("RejectPeginProcessor completing GetCommInfo for flow {flow_id}");
                        flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                debug!("RejectPeginProcessor received SetupCompleted for {program_id}");
                if let Some(flow) = self.flows.get_mut(program_id) {
                    if flow.current_step() == Steps::SendRejectPegin {
                        flow.complete_step(StepData::SetupCompleted)?;
                    } else {
                        trace!(
                            "RejectPeginProcessor ignoring SetupCompleted for flow {} in step {:?}",
                            program_id,
                            flow.current_step()
                        );
                    }
                    cleanup_flows_matching(
                        self.store.as_ref(),
                        StorePrefix::RejectPeginFlow,
                        &mut self.flows,
                        RejectPeginFlow::is_done,
                    );
                } else {
                    trace!("RejectPeginProcessor ignoring SetupCompleted for unknown {program_id}");
                }
            }
            OutgoingBitVMXApiMessages::Transaction(program_id, tx_status, label) => {
                if label.as_deref() == Some(REJECT_PEGIN_TX) {
                    self.handle_reject_pegin_tx_received(*program_id, tx_status)?;
                }
            }
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => {
                if let Some(spv_proof) = spv_proof_opt {
                    self.handle_spv_proof(*tx_id, spv_proof.clone())?;
                } else {
                    return Err(anyhow!(
                        "RejectPeginProcessor: SPVProof for tx_id {tx_id} has no proof"
                    ));
                }
            }
            _ => {
                trace!("RejectPeginProcessor ignoring BitVMX event {event:?}");
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        if self.config.required_confirmations == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) =
            if let RskPegManagerEvents::RejectPeginRegistered(e) = event {
                if !self.has_flow_for_reject_pegin_registered(e) {
                    trace!(
                        "RejectPeginProcessor ignoring RejectPeginRegistered - no matching flow",
                    );
                    return Ok(());
                }
                Self::build_reject_pegin_registered_event_info(e)
            } else {
                trace!("RejectPeginProcessor ignoring RSK event {event:?}");
                return Ok(());
            };

        if is_removal {
            warn!("Removing pending reject pegin RSK event {event:?}");
            if let Some(mut removed_event) = self.events_confirming.remove(&id) {
                if let Err(err) = removed_event.stop_confirming() {
                    warn!("Failed to stop confirming removed reject pegin event {id}: {err}");
                }
            } else {
                warn!("Tried to remove non-existing pending reject pegin event with id {id}");
            }
        } else {
            debug!(
                "Adding pending reject pegin event {event:?}, start confirming at block {block_num}",
            );

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                self.config.required_confirmations,
                self.blockchain_view.clone(),
                managed_event,
            );
            confirmable_event
                .start_confirming(block_num)
                .context("Starting confirming reject pegin event")?;
            self.events_confirming.insert(confirmable_event.id(), confirmable_event);
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.process_block_confirmations(block)?;
        self.handle_transaction_status_tick()?;
        self.handle_register_reject_pegin_retry_tick();
        Ok(())
    }

    fn shutdown(&mut self) {
        self.flows.clear();
        self.events_confirming.clear();
        self.tx_status_scheduler.clear();
        self.unconfirmed_register_reject_pegin.clear();
        self.register_reject_pegin_retry_scheduler.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::rc::Rc;

    use bitcoin::Transaction;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use common::msg_broker::bitvmx_types::{
        BtcTxSPVProof, CommsAddress, IncomingBitVMXApiMessages, REJECT_PEGIN_TX,
        TransactionBlockchainStatus, TransactionStatus,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::runtime_sync::RuntimeSync;
    use common::types::{CommitteeId, RskBlockAndUncles};
    use mockall::Sequence;
    use primitive_types::U256 as RskU256;
    use transaction_dispatcher::rsk_gateway::DomainErrors;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
    use crate::store::CoordinatorStore;
    use crate::types::RejectPeginRegisteredData;
    use crate::user_requests::RejectPeginRequest;

    fn create_fake_block(
        number: common::types::BlockNumber,
        effort: RskU256,
    ) -> common::types::RskBlock {
        use common::types::{BlockHash, BlockPow, RskBlock};
        use primitive_types::H256;
        let block_pow_u = RskU256::MAX.checked_div(effort).expect("non-zero effort");
        let pow = BlockPow::from(H256::from_slice(&block_pow_u.to_big_endian()));
        let block_hash = BlockHash::from(H256::from_low_u64_be(number.value()));
        let parent_hash = BlockHash::from(H256::from_low_u64_be(number.value().saturating_sub(1)));
        RskBlock::new(
            number,
            block_hash,
            parent_hash,
            common::types::BlockTimestamp::from(number.value() * 1000),
            common::types::BlockDifficulty::from(RskU256::from(500_u64)),
            common::types::BlockDifficulty::from(RskU256::from(500_u64 * number.value())),
            pow,
            vec![],
        )
    }

    type BitVmxMock = MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;
    type TestProcessor =
        RejectPeginProcessor<MockRskContractsGatewayApi, BitVmxMock, CoordinatorStore>;

    fn test_request() -> RejectPeginRequest {
        RejectPeginRequest {
            committee_id: CommitteeId::from(77_u128),
            member_index: 1,
            request_pegin_txid: "1111111111111111111111111111111111111111111111111111111111111111"
                .parse()
                .expect("valid txid"),
        }
    }

    fn test_global_context() -> GlobalContext {
        let context = GlobalContext::new();
        let request = test_request();
        context.my_committees().add(
            request.committee_id.clone(),
            common::msg_broker::bitvmx_types::ParticipantRole::Verifier,
        );
        context
    }

    fn test_store() -> Rc<CoordinatorStore> {
        let path =
            std::env::temp_dir().join(format!("reject-pegin-processor-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp store dir");
        Rc::new(CoordinatorStore::new(path.to_str().expect("utf8 path")).expect("store"))
    }

    fn test_processor_with(
        bitvmx_broker: BitVmxMock,
        contracts_gateway: MockRskContractsGatewayApi,
        global_context: GlobalContext,
        config: RejectPeginProcessorConfig,
    ) -> TestProcessor {
        test_processor_with_store(
            bitvmx_broker,
            contracts_gateway,
            global_context,
            &test_store(),
            config,
        )
    }

    fn test_processor_with_store(
        bitvmx_broker: BitVmxMock,
        contracts_gateway: MockRskContractsGatewayApi,
        global_context: GlobalContext,
        store: &Rc<CoordinatorStore>,
        config: RejectPeginProcessorConfig,
    ) -> TestProcessor {
        RejectPeginProcessor::new(
            Rc::new(bitvmx_broker),
            Rc::new(contracts_gateway),
            RuntimeSync::new().expect("runtime"),
            global_context,
            store,
            config,
            NativeBridgeVerifier::Dummy,
        )
    }

    fn test_processor(bitvmx_broker: BitVmxMock, global_context: GlobalContext) -> TestProcessor {
        test_processor_with(
            bitvmx_broker,
            MockRskContractsGatewayApi::new(),
            global_context,
            RejectPeginProcessorConfig::default(),
        )
    }

    fn test_tx_status(confirmations: u32) -> TransactionStatus {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        TransactionStatus {
            tx_id: tx.compute_txid(),
            tx,
            block_info: None,
            confirmations,
            status: TransactionBlockchainStatus::Confirmed,
        }
    }

    fn test_spv_proof(tx: &Transaction) -> BtcTxSPVProof {
        BtcTxSPVProof {
            block_hash: "00".repeat(32),
            tx: tx.clone(),
            merkle_branch_path: "0".to_string(),
            merkle_branch_hashes: vec![],
        }
    }

    fn test_reject_pegin_registered(
        reject_pegin_txid: Txid,
        request_pegin_txid: Txid,
    ) -> RejectPeginRegisteredEvent {
        RejectPeginRegisteredEvent {
            inner: RejectPeginRegisteredData {
                reject_pegin_txid,
                request_pegin_txid,
                stream_id: 42,
                packet_number: 33,
                slot_id: 1,
                peg_status: 2,
            },
            block_number: 100.into(),
            block_hash: common::types::BlockHash::from(primitive_types::H256::from_low_u64_be(1)),
            removed: false,
            tx_hash: common::types::TxHash::from(primitive_types::H256::from_low_u64_be(2)),
        }
    }

    #[test]
    fn process_user_request_starts_reject_pegin_flow() {
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker.expect_send().times(1).returning(|_| Ok(true));

        let mut processor = test_processor(bitvmx_broker, test_global_context());

        processor
            .process_user_request(&UserRequests::RejectPegin(test_request()))
            .expect("reject pegin request is handled");

        assert_eq!(processor.active_flows_len(), 1);
    }

    #[test]
    fn process_new_bitvmx_event_ignores_non_matching_setup_completed() {
        let bitvmx_broker = BitVmxMock::new();
        let mut processor = test_processor(bitvmx_broker, test_global_context());

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SetupCompleted(Uuid::new_v4()))
            .expect("unknown setup completed is ignored");

        assert_eq!(processor.active_flows_len(), 0);
    }

    #[test]
    fn process_new_bitvmx_event_after_setup_waits_for_reject_pegin_tx() {
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker.expect_send().times(3).returning(|_| Ok(true));

        let mut processor = test_processor(bitvmx_broker, test_global_context());

        processor
            .process_user_request(&UserRequests::RejectPegin(test_request()))
            .expect("reject pegin request is handled");
        let mut flow_ids = processor.active_flow_ids();
        let protocol_id = flow_ids.pop().expect("one active flow");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::CommInfo(
                Uuid::new_v4(),
                CommsAddress {
                    address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 61180),
                    pubkey_hash: "cd".repeat(32),
                },
            ))
            .expect("comm info is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SetupCompleted(protocol_id))
            .expect("matching setup completed is handled");

        // After setup we wait for REJECT_PEGIN_TX, flow is not removed yet
        assert_eq!(processor.active_flows_len(), 1);
    }

    #[test]
    fn restored_processor_rehydrates_tx_status_polling_for_reject_pegin_flow() {
        let store = test_store();
        let global_context = test_global_context();
        let config = RejectPeginProcessorConfig {
            min_tx_confirmations: 2,
            blocks_delay_for_tx_check: 5,
            required_confirmations: 1,
        };

        let mut initial_broker = BitVmxMock::new();
        initial_broker.expect_send().times(3).returning(|_| Ok(true));

        let tx_status = test_tx_status(1);
        let protocol_id = {
            let mut processor = test_processor_with_store(
                initial_broker,
                MockRskContractsGatewayApi::new(),
                global_context.clone(),
                &store,
                config.clone(),
            );

            processor
                .process_user_request(&UserRequests::RejectPegin(test_request()))
                .expect("reject pegin request is handled");
            let protocol_id = processor.active_flow_ids().pop().expect("one active flow");

            processor
                .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::CommInfo(
                    Uuid::new_v4(),
                    CommsAddress {
                        address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 61180),
                        pubkey_hash: "cd".repeat(32),
                    },
                ))
                .expect("comm info is handled");

            processor
                .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SetupCompleted(protocol_id))
                .expect("setup completed is handled");

            processor
                .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::Transaction(
                    protocol_id,
                    tx_status.clone(),
                    Some(REJECT_PEGIN_TX.to_string()),
                ))
                .expect("transaction status is handled");

            assert_eq!(
                processor.flows.get(&protocol_id).and_then(RejectPeginFlow::get_reject_pegin_txid),
                Some(tx_status.tx_id)
            );
            assert!(processor.tx_status_scheduler.is_scheduled(&protocol_id));

            protocol_id
        };

        let mut restored_broker = BitVmxMock::new();
        restored_broker
            .expect_send()
            .with(mockall::predicate::function(move |msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::GetTransaction(id, tx_id)
                        if *id == protocol_id && *tx_id == tx_status.tx_id
                )
            }))
            .times(1)
            .returning(|_| Ok(true));

        let mut restored_processor = test_processor_with_store(
            restored_broker,
            MockRskContractsGatewayApi::new(),
            global_context,
            &store,
            config,
        );

        assert_eq!(restored_processor.active_flows_len(), 1);
        assert_eq!(
            restored_processor
                .flows
                .get(&protocol_id)
                .and_then(RejectPeginFlow::get_reject_pegin_txid),
            Some(tx_status.tx_id)
        );
        assert!(restored_processor.tx_status_scheduler.is_scheduled(&protocol_id));

        restored_processor
            .handle_transaction_status_tick()
            .expect("restored flow should request transaction status");
    }

    #[test]
    fn spv_proof_retries_reject_pegin_registration_when_native_bridge_confirmations_are_missing() {
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker.expect_send().times(4).returning(|_| Ok(true));

        let mut contracts_gateway = MockRskContractsGatewayApi::new();
        let mut sequence = Sequence::new();
        contracts_gateway.expect_reject_pegin().times(1).in_sequence(&mut sequence).returning(
            |_| {
                Err(DomainErrors::MissingConfirmationsOnNativeBridge(
                    "missing confirmations".to_string(),
                ))
            },
        );
        contracts_gateway.expect_reject_pegin().times(1).in_sequence(&mut sequence).returning(
            |_| {
                Ok(transaction_dispatcher::types::TxSentOutput {
                    transaction_hash: "0xdeadbeef".to_string(),
                })
            },
        );

        let config = RejectPeginProcessorConfig {
            min_tx_confirmations: 1,
            blocks_delay_for_tx_check: 0,
            required_confirmations: 5,
        };
        let mut processor =
            test_processor_with(bitvmx_broker, contracts_gateway, test_global_context(), config);

        processor
            .process_user_request(&UserRequests::RejectPegin(test_request()))
            .expect("reject pegin request is handled");
        let protocol_id = processor.active_flow_ids().pop().expect("one active flow");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::CommInfo(
                Uuid::new_v4(),
                CommsAddress {
                    address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 61180),
                    pubkey_hash: "cd".repeat(32),
                },
            ))
            .expect("comm info is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SetupCompleted(protocol_id))
            .expect("setup completed is handled");

        let tx_status = test_tx_status(1);
        let spv_proof = test_spv_proof(&tx_status.tx);
        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::Transaction(
                protocol_id,
                tx_status.clone(),
                Some(REJECT_PEGIN_TX.to_string()),
            ))
            .expect("transaction status is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SPVProof(
                tx_status.tx_id,
                Some(spv_proof),
            ))
            .expect("spv proof schedules retry on missing native bridge confirmations");

        assert_eq!(processor.active_flows_len(), 1);

        processor.handle_register_reject_pegin_retry_tick();

        assert_eq!(processor.active_flows_len(), 1);
        assert_eq!(
            processor.flows.get(&protocol_id).map(RejectPeginFlow::current_step),
            Some(Steps::RegisterRejectPeginSpv)
        );
    }

    #[test]
    fn reject_pegin_registered_finishes_flow_after_rsk_confirmation() {
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker.expect_send().times(4).returning(|_| Ok(true));

        let mut contracts_gateway = MockRskContractsGatewayApi::new();
        contracts_gateway.expect_reject_pegin().times(1).returning(|_| {
            Ok(transaction_dispatcher::types::TxSentOutput {
                transaction_hash: "0xdeadbeef".to_string(),
            })
        });

        let config = RejectPeginProcessorConfig {
            min_tx_confirmations: 1,
            blocks_delay_for_tx_check: 0,
            required_confirmations: 1,
        };
        let mut processor =
            test_processor_with(bitvmx_broker, contracts_gateway, test_global_context(), config);

        processor
            .process_user_request(&UserRequests::RejectPegin(test_request()))
            .expect("reject pegin request is handled");
        let protocol_id = processor.active_flow_ids().pop().expect("one active flow");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::CommInfo(
                Uuid::new_v4(),
                CommsAddress {
                    address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 61180),
                    pubkey_hash: "cd".repeat(32),
                },
            ))
            .expect("comm info is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SetupCompleted(protocol_id))
            .expect("setup completed is handled");

        let tx_status = test_tx_status(1);
        let spv_proof = test_spv_proof(&tx_status.tx);
        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::Transaction(
                protocol_id,
                tx_status.clone(),
                Some(REJECT_PEGIN_TX.to_string()),
            ))
            .expect("transaction status is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SPVProof(
                tx_status.tx_id,
                Some(spv_proof),
            ))
            .expect("spv proof registers reject pegin");

        assert_eq!(
            processor.flows.get(&protocol_id).map(RejectPeginFlow::current_step),
            Some(Steps::RegisterRejectPeginSpv)
        );

        processor
            .process_new_rsk_event(&RskPegManagerEvents::RejectPeginRegistered(
                test_reject_pegin_registered(tx_status.tx_id, test_request().request_pegin_txid),
            ))
            .expect("RejectPeginRegistered event is queued for confirmation");

        assert_eq!(processor.active_flows_len(), 1);

        let block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(101.into(), RskU256::from(50)));
        processor.process_new_block(&block).expect("block confirmations are handled");

        assert_eq!(processor.active_flows_len(), 0);
    }

    #[test]
    fn reject_pegin_registered_still_waits_for_rsk_confirmation_when_btc_confirmations_disabled() {
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker.expect_send().times(4).returning(|_| Ok(true));

        let mut contracts_gateway = MockRskContractsGatewayApi::new();
        contracts_gateway.expect_reject_pegin().times(1).returning(|_| {
            Ok(transaction_dispatcher::types::TxSentOutput {
                transaction_hash: "0xdeadbeef".to_string(),
            })
        });

        let config = RejectPeginProcessorConfig {
            min_tx_confirmations: 0,
            blocks_delay_for_tx_check: 0,
            required_confirmations: 1,
        };
        let mut processor =
            test_processor_with(bitvmx_broker, contracts_gateway, test_global_context(), config);

        processor
            .process_user_request(&UserRequests::RejectPegin(test_request()))
            .expect("reject pegin request is handled");
        let protocol_id = processor.active_flow_ids().pop().expect("one active flow");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::CommInfo(
                Uuid::new_v4(),
                CommsAddress {
                    address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 61180),
                    pubkey_hash: "cd".repeat(32),
                },
            ))
            .expect("comm info is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SetupCompleted(protocol_id))
            .expect("setup completed is handled");

        let tx_status = test_tx_status(0);
        let spv_proof = test_spv_proof(&tx_status.tx);
        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::Transaction(
                protocol_id,
                tx_status.clone(),
                Some(REJECT_PEGIN_TX.to_string()),
            ))
            .expect("transaction status is handled");

        processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::SPVProof(
                tx_status.tx_id,
                Some(spv_proof),
            ))
            .expect("spv proof registers reject pegin");

        processor
            .process_new_rsk_event(&RskPegManagerEvents::RejectPeginRegistered(
                test_reject_pegin_registered(tx_status.tx_id, test_request().request_pegin_txid),
            ))
            .expect("RejectPeginRegistered event is queued for confirmation");

        assert_eq!(processor.active_flows_len(), 1);
        assert_eq!(processor.events_confirming.len(), 1);

        let block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(101.into(), RskU256::from(50)));
        processor.process_new_block(&block).expect("block confirmations are handled");

        assert_eq!(processor.active_flows_len(), 0);
        assert!(processor.events_confirming.is_empty());
    }
}
