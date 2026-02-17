//! Setup committee processor: routes events and user requests to the appropriate
//! [`SetupCommitteeFlow`](super::setup_committee_flow::SetupCommitteeFlow) instances.
//!
//! This module is intentionally decoupled from the flow implementation; it only
//! uses the flow's public API and the factory trait.

use std::any::type_name_of_val;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::{
    GLOBAL_SETTINGS_UUID, IncomingBitVMXApiMessages, OP_COSIGN_UTXOS, OutgoingBitVMXApiMessages,
    UnionSettings, VariableTypes, WT_INIT_CHALLENGE_UTXOS,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::types::{BlockNumber, CommitteeId, RskBlockAndUncles, StreamId};
use log::{debug, error, info, trace, warn};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use super::setup_committee_flow::{
    SetupCommitteeFlow, SetupCommitteeFlowApi, SetupCommitteeFlowFactoryApi, State, StepData, Steps,
};
use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::event_processor::EventProcessor;
use crate::flows::common::GlobalContext;
use crate::flows::errors::{FailableFlow, FlowError};
use crate::store::{CoordinatorStoreApi, StoreKey, StorePrefix, cleanup_completed_flows, restore_flows};
use crate::types::{
    AllCommunicationDataReadyEvent, EventStatus, NewCommitteePendingEvent, NewCommitteeReadyEvent,
    RskPegManagerEvents, UserRequests,
};

pub(crate) struct SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi,
{
    flow_factory: FactoryBSF,
    flows: HashMap<Uuid, SetupCommitteeFlow<CG, BC, S>>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    store: Rc<S>,
    required_confirmations: u32,
}

impl<CG, BC, FactoryBSF, S> SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi + 'static,
{
    pub(crate) fn new(
        flow_factory: FactoryBSF,
        global_context: GlobalContext,
        store: &Rc<S>,
        bitvmx_broker: &BC,
        required_confirmations: u32,
    ) -> Self {
        // Send global UnionSettings to BitVMX (once at startup)
        Self::send_union_settings(bitvmx_broker).expect("Failed to send UnionSettings to BitVMX");

        info!("Successfully sent UnionSettings to BitVMX");

        let mut processor = Self {
            flow_factory,
            flows: HashMap::new(),
            global_context,
            events_confirming: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            store: Rc::clone(store),
            required_confirmations,
        };

        let flow_factory =
            |saved_state: State| processor.flow_factory.create_flow_from_saved_state(saved_state);

        processor.flows =
            restore_flows(store.as_ref(), StorePrefix::SetupCommitteeFlow, flow_factory)
                .expect("Failed to load flows from store");
        processor
    }

    fn send_union_settings(bitvmx_broker: &BC) -> Result<()> {
        let settings = UnionSettings::with_defaults();
        let settings_json = serde_json::to_string(&settings)?;

        bitvmx_broker.send(IncomingBitVMXApiMessages::SetVar(
            GLOBAL_SETTINGS_UUID,
            UnionSettings::name(),
            VariableTypes::String(settings_json),
        ))?;

        Ok(())
    }
}

impl<CG, BC, FactoryBSF, S> SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi + 'static,
{
    fn dispatch_to_flow(&mut self, req_id: &Uuid, step_data: StepData) {
        if let Some(flow) = self.get_flow_for_bitvmx_response(req_id) {
            Self::continue_flow(flow, step_data);
        } else {
            debug!("No flow found for BitVMX event with id {req_id}");
        }
    }

    fn dispatch_to_flow_by_step(&mut self, expected_step: Steps, step_data: StepData) {
        if let Some(flow) = self.flows.values_mut().find(|f| f.current_step() == expected_step) {
            Self::continue_flow(flow, step_data);
        } else {
            trace!("No flow found in step {expected_step:?}");
        }
    }

    fn dispatch_to_flow_by_program_id(&mut self, program_id: &Uuid, step_data: StepData) {
        if let Some(flow) = self
            .flows
            .values_mut()
            .find(|flow| flow.is_waiting_for_dispute_core_variable(program_id))
        {
            Self::continue_flow(flow, step_data);
        } else {
            debug!("No flow in RequestDisputeChannelVars step for DisputeCore pid {program_id}");
        }
    }

    fn continue_flow(flow: &mut SetupCommitteeFlow<CG, BC, S>, data: StepData) {
        let internal_id = flow.internal_id();
        let current_step = flow.current_step();
        trace!("Continuing flow {internal_id} at step {current_step:?} with data: {data:?}");
        match flow.complete_step(data) {
            Ok(()) => {
                trace!(
                    "Step {:?} completed successfully for flow {}",
                    flow.current_step(),
                    flow.internal_id()
                );
            }
            Err(FlowError::Fatal { message, source }) => {
                error!("Fatal error in flow {internal_id} at step {current_step:?}: {message}");
                Self::log_flow_error_source("Fatal", internal_id, current_step, source.as_ref());
                flow.fail();
            }
            Err(FlowError::Transient { message, source }) => {
                error!("Transient error in flow {internal_id} at step {current_step:?}: {message}");
                Self::log_flow_error_source(
                    "Transient",
                    internal_id,
                    current_step,
                    source.as_ref(),
                );
            }
        }
        debug!(
            "Completed continue_flow at step {:?} for flow {}",
            flow.current_step(),
            flow.internal_id()
        );
    }

    fn log_flow_error_source(
        kind: &str,
        internal_id: Uuid,
        step: Steps,
        source: Option<&anyhow::Error>,
    ) {
        if let Some(err) = source {
            let chain =
                err.chain().map(std::string::ToString::to_string).collect::<Vec<_>>().join(" | ");
            error!("{kind} error source chain for flow {internal_id} at step {step:?}: {chain}");
        }
    }

    fn get_flow_for_stream_id(
        &mut self,
        stream_id: &StreamId,
        expected_step: Steps,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        self.flows
            .values_mut()
            .find(|f| f.current_step() == expected_step && f.is_for_stream(stream_id))
    }

    fn get_flow_for_committee_pending(
        &mut self,
        committee_id: &CommitteeId,
        expected_step: Steps,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        if !self.global_context.my_committees().im_member(committee_id) {
            debug!("Skipping committee {committee_id} - not mine");
            return None;
        }

        let pending_committee_flows: Vec<_> = self
            .flows
            .values_mut()
            .filter(|f| f.current_step() == expected_step && f.is_for_committee(committee_id))
            .collect();

        if pending_committee_flows.len() > 1 {
            error!("Multiple flows in status committee_pending for committee {committee_id}");
            None
        } else {
            pending_committee_flows.into_iter().next()
        }
    }

    fn get_flow_for_bitvmx_response(
        &mut self,
        req_id: &Uuid,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        debug!("Getting flow for bitvmx response {req_id:?}");
        self.flows.values_mut().find(|flow| flow.is_waiting_for_bitvmx_request(req_id))
    }

    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) {
        info!("Processing confirmed RSK event: {event:?}");
        let flow_data = match event {
            RskPegManagerEvents::NewCommitteePending(ncp) => {
                let stream_id: StreamId = ncp.inner._committee.streamId.into();
                let found_flow = self.get_flow_for_stream_id(&stream_id, Steps::ApplyToStream);
                found_flow.map(|f| (f, StepData::PendingCommittee(ncp.clone())))
            }
            RskPegManagerEvents::AllCommunicationDataReady(acdr) => {
                let committee_id: CommitteeId = acdr.inner._committeeId.into();
                let found_flow =
                    self.get_flow_for_committee_pending(&committee_id, Steps::DepositP2PData);
                found_flow.map(|f| (f, StepData::ReadyCommunicationData(acdr.clone())))
            }
            RskPegManagerEvents::NewCommitteeReady(ncr) => {
                let committee_id: CommitteeId = ncr.inner.committeeId.into();
                let found_flow =
                    self.get_flow_for_committee_pending(&committee_id, Steps::DepositAggregatedKey);
                found_flow.map(|f| (f, StepData::ReadyCommittee(ncr.clone())))
            }
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return;
            }
        };

        match flow_data {
            Some((flow, step_data)) => {
                Self::continue_flow(flow, step_data);
            }
            None => {
                warn!("Received {event:?} but no matching flow found");
            }
        }
    }

    fn build_new_committee_ready_event_info(
        event: &NewCommitteeReadyEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("{}-ready", event.inner.committeeId),
            event.removed,
            event.block_number,
            RskPegManagerEvents::NewCommitteeReady(event.clone()),
        )
    }
    fn build_all_comm_data_ready_event_info(
        event: &AllCommunicationDataReadyEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("{}-data-ready", event.inner._committeeId),
            event.removed,
            event.block_number,
            RskPegManagerEvents::AllCommunicationDataReady(event.clone()),
        )
    }
    fn build_new_pending_committee_event_info(
        event: &NewCommitteePendingEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("{}-pending", event.inner.committeeId),
            event.removed,
            event.block_number,
            RskPegManagerEvents::NewCommitteePending(event.clone()),
        )
    }
}

impl<CG, BC, FactoryBSF, S> EventProcessor for SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi + 'static,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        info!("Processing user request: {req:?}");
        match req {
            UserRequests::ApplyToStream(input) => {
                let internal_id = Uuid::new_v4();
                let mut flow = self.flow_factory.create_flow(internal_id);

                Self::continue_flow(&mut flow, StepData::UserRequest(input.clone()));

                self.flows.insert(internal_id, flow);
            }
            UserRequests::GetBitVMXFundingAddress => {
                trace!("Ignoring user request: {req:?}");
            }
        }
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        debug!("Processing new bitvmx event: {event:?}");

        match event {
            OutgoingBitVMXApiMessages::CommInfo(_req_id, comm_info) => {
                self.dispatch_to_flow_by_step(
                    Steps::GetMyCommInfo,
                    StepData::CommInfo(comm_info.clone()),
                );
            }

            OutgoingBitVMXApiMessages::AggregatedPubkey(req_id, pubkey) => {
                info!("PK Received AggregatedPubkey: {req_id:?}, {pubkey:?}");
                if let Some(flow) = self.get_flow_for_bitvmx_response(req_id) {
                    let is_pairwise = flow.is_pairwise_aggregated_key_request(req_id);
                    let step_data = if is_pairwise {
                        StepData::PairwiseAggregatedKey(*req_id, *pubkey)
                    } else {
                        StepData::PublicKey(*pubkey)
                    };
                    Self::continue_flow(flow, step_data);
                } else {
                    debug!("No flow found for AggregatedPubkey with id {req_id}");
                }
            }

            OutgoingBitVMXApiMessages::Variable(program_id, var_name, var_value)
                if matches!(var_name.as_str(), OP_COSIGN_UTXOS | WT_INIT_CHALLENGE_UTXOS) =>
            {
                let VariableTypes::String(json_str) = var_value else {
                    warn!("Received DisputeCore variable with unexpected type: {var_value:?}");
                    return Ok(());
                };
                debug!("Received DisputeCore variable {var_name} from program_id: {program_id}");
                self.dispatch_to_flow_by_program_id(
                    program_id,
                    StepData::DisputeCoreVariable(*program_id, var_name.clone(), json_str.clone()),
                );
            }

            OutgoingBitVMXApiMessages::AggregatedPubkeyNotReady(req_id) => {
                anyhow::bail!("BitVMX cannot aggregate dispute keys for request {req_id}")
            }
            OutgoingBitVMXApiMessages::WalletError(req_id, tx_id) => {
                anyhow::bail!("BitVMX WalletError for request {req_id}, tx {tx_id}")
            }
            OutgoingBitVMXApiMessages::WalletNotReady(req_id) => {
                anyhow::bail!("BitVMX WalletNotReady for request {req_id}")
            }

            OutgoingBitVMXApiMessages::FundingBalance(req_id, balance) => {
                self.dispatch_to_flow(req_id, StepData::BitVmxFundingBalance(*balance));
            }
            OutgoingBitVMXApiMessages::PubKey(req_id, public_key) => {
                info!("PK Received PubKey: {req_id:?}, {public_key:?}");
                self.dispatch_to_flow(req_id, StepData::PublicKey(*public_key));
            }
            OutgoingBitVMXApiMessages::SignedMessage(sign_req_id, r, s, rec_id) => {
                self.dispatch_to_flow(sign_req_id, StepData::SignedMessage(*r, *s, *rec_id));
            }
            OutgoingBitVMXApiMessages::SetupCompleted(req_id) => {
                self.dispatch_to_flow(req_id, StepData::SetupCompleted(*req_id));
            }
            OutgoingBitVMXApiMessages::FundsSent(req_id, tx_id) => {
                self.dispatch_to_flow(req_id, StepData::FundsSent(*tx_id));
            }

            OutgoingBitVMXApiMessages::Pong(_) => {}
            _ => {
                trace!("Ignoring BitVMX event: {}", type_name_of_val(event));
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        debug!("Processing new rsk event: {event:?}");
        if self.required_confirmations == 0 {
            self.process_confirmed_rsk_event(event);
            return Ok(());
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::NewCommitteePending(e) => {
                Self::build_new_pending_committee_event_info(e)
            }
            RskPegManagerEvents::AllCommunicationDataReady(e) => {
                Self::build_all_comm_data_ready_event_info(e)
            }
            RskPegManagerEvents::NewCommitteeReady(e) => {
                Self::build_new_committee_ready_event_info(e)
            }
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if is_removal {
            warn!("Removing pending RSK event: {event:?}");

            if let Some(mut removed_ev) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_ev.stop_confirming() {
                    error!("Failed to stop confirming for removed event {id}: {e}");
                }
            } else {
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
        if self.events_confirming.is_empty() {
            trace!("No events left to confirm, skipping block");
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
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                if let Err(e) = event.stop_confirming() {
                    error!("Failed to stop confirming for event {key}: {e}");
                }
                self.process_confirmed_rsk_event(event.get_data());
            }
        }

        if self.events_confirming.is_empty() {
            debug!("No events left to confirm, clearing blockchain view");
            self.blockchain_view.clear();
        }

        // blocks allow periodic cleanup of completed flows, we can improve it with a cleanup task if needed
        cleanup_completed_flows(
            self.store.as_ref(),
            StorePrefix::SetupCommitteeFlow,
            &mut self.flows,
            SetupCommitteeFlow::is_done,
        );
        Ok(())
    }

    fn shutdown(&mut self) {
        // TODO handle shutdown logic if necessary
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use alloy_primitives::{Address as AlloyAddress, Bytes, U256};
    use common::msg_broker::bitvmx_types::{
        GLOBAL_SETTINGS_UUID, IncomingBitVMXApiMessages, OP_COSIGN_UTXOS,
        OutgoingBitVMXApiMessages, UnionSettings, VariableTypes,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::types::{BlockHash, BlockNumber, TxHash};
    use mockall::predicate::function;
    use primitive_types::H256;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        AllCommunicationDataReady, Committee, CommitteeMember, NewCommittee, NewPendingCommittee,
    };
    use uuid::Uuid;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::store::MockCoordinatorStoreApi;
    use crate::types::{EventWithBlock, RskPegManagerEvents};

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;
    type TestProcessor = SetupCommitteeProcessor<
        MockRskContractsGatewayApi,
        MockBitVmxBroker,
        DummyFactory,
        MockCoordinatorStoreApi,
    >;

    #[derive(Clone, Default)]
    pub(crate) struct DummyFactory;

    impl
        SetupCommitteeFlowFactoryApi<
            MockRskContractsGatewayApi,
            MockBitVmxBroker,
            MockCoordinatorStoreApi,
        > for DummyFactory
    {
        fn create_flow(
            &self,
            _internal_id: Uuid,
        ) -> SetupCommitteeFlow<MockRskContractsGatewayApi, MockBitVmxBroker, MockCoordinatorStoreApi>
        {
            panic!("create_flow should not be called in this test");
        }

        fn create_flow_from_saved_state(
            &self,
            _saved_state: State,
        ) -> SetupCommitteeFlow<MockRskContractsGatewayApi, MockBitVmxBroker, MockCoordinatorStoreApi>
        {
            panic!("create_flow_from_saved_state should not be called in this test");
        }
    }

    fn test_committee(member_address: AlloyAddress) -> Committee {
        Committee {
            aggregatedKey: Bytes::from(vec![1u8; 32]),
            members: vec![CommitteeMember { memberAddress: member_address, role: 1 }],
            leaderAddress: member_address,
            operatorTakeIndex: U256::from(0),
            createdAt: U256::from(0),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 1,
            fundingUTXOs: vec![],
        }
    }

    fn test_pending_event(
        committee_id: u128,
        removed: bool,
    ) -> EventWithBlock<NewPendingCommittee> {
        let member: AlloyAddress = [1u8; 20].into();
        EventWithBlock {
            inner: NewPendingCommittee {
                committeeId: committee_id,
                _committee: test_committee(member),
            },
            block_number: BlockNumber::from(10),
            block_hash: BlockHash::from(H256::from_low_u64_be(11)),
            removed,
            tx_hash: TxHash::from(H256::from_low_u64_be(12)),
        }
    }

    fn test_ready_event(committee_id: u128, removed: bool) -> EventWithBlock<NewCommittee> {
        let member: AlloyAddress = [2u8; 20].into();
        EventWithBlock {
            inner: NewCommittee { committeeId: committee_id, _committee: test_committee(member) },
            block_number: BlockNumber::from(20),
            block_hash: BlockHash::from(H256::from_low_u64_be(21)),
            removed,
            tx_hash: TxHash::from(H256::from_low_u64_be(22)),
        }
    }

    fn test_data_ready_event(
        committee_id: u128,
        removed: bool,
    ) -> EventWithBlock<AllCommunicationDataReady> {
        EventWithBlock {
            inner: AllCommunicationDataReady { _committeeId: committee_id },
            block_number: BlockNumber::from(30),
            block_hash: BlockHash::from(H256::from_low_u64_be(31)),
            removed,
            tx_hash: TxHash::from(H256::from_low_u64_be(32)),
        }
    }

    fn empty_processor() -> TestProcessor {
        SetupCommitteeProcessor {
            flow_factory: DummyFactory,
            flows: HashMap::new(),
            global_context: GlobalContext::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            store: Rc::new(MockCoordinatorStoreApi::new()),
            required_confirmations: 1,
        }
    }

    #[test]
    fn test_send_union_settings_sends_global_set_var_message() {
        let mut broker = MockBitVmxBroker::new();
        broker
            .expect_send()
            .with(function(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(id, name, VariableTypes::String(payload))
                        if *id == GLOBAL_SETTINGS_UUID
                            && *name == UnionSettings::name()
                            && serde_json::from_str::<UnionSettings>(payload).is_ok()
                )
            }))
            .times(1)
            .returning(|_| Ok(true));

        let result = TestProcessor::send_union_settings(&broker);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_new_pending_committee_event_info_includes_pending_suffix() {
        let event = test_pending_event(123, true);
        let (id, removed, block_number, wrapped) =
            TestProcessor::build_new_pending_committee_event_info(&event);

        assert_eq!(id, "123-pending");
        assert!(removed);
        assert_eq!(block_number, event.block_number);
        assert!(matches!(wrapped, RskPegManagerEvents::NewCommitteePending(_)));
    }

    #[test]
    fn test_build_all_comm_data_ready_event_info_includes_data_ready_suffix() {
        let event = test_data_ready_event(321, false);
        let (id, removed, block_number, wrapped) =
            TestProcessor::build_all_comm_data_ready_event_info(&event);

        assert_eq!(id, "321-data-ready");
        assert!(!removed);
        assert_eq!(block_number, event.block_number);
        assert!(matches!(wrapped, RskPegManagerEvents::AllCommunicationDataReady(_)));
    }

    #[test]
    fn test_build_new_committee_ready_event_info_includes_ready_suffix() {
        let event = test_ready_event(777, false);
        let (id, removed, block_number, wrapped) =
            TestProcessor::build_new_committee_ready_event_info(&event);

        assert_eq!(id, "777-ready");
        assert!(!removed);
        assert_eq!(block_number, event.block_number);
        assert!(matches!(wrapped, RskPegManagerEvents::NewCommitteeReady(_)));
    }

    #[test]
    fn test_process_new_bitvmx_event_wallet_not_ready_returns_error() {
        let mut processor = empty_processor();
        let req_id = Uuid::new_v4();

        let err = processor
            .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::WalletNotReady(req_id))
            .expect_err("wallet not ready should return error");

        assert!(err.to_string().contains("WalletNotReady"));
    }

    #[test]
    fn test_process_new_bitvmx_event_pong_is_ignored() {
        let mut processor = empty_processor();
        assert!(
            processor
                .process_new_bitvmx_event(&OutgoingBitVMXApiMessages::Pong(Uuid::new_v4()))
                .is_ok()
        );
    }

    #[test]
    fn test_process_new_bitvmx_event_non_string_dispute_var_is_ignored() {
        let mut processor = empty_processor();
        let result = processor.process_new_bitvmx_event(&OutgoingBitVMXApiMessages::Variable(
            Uuid::new_v4(),
            OP_COSIGN_UTXOS.to_string(),
            VariableTypes::Number(42),
        ));
        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_ignored_event_returns_ok() {
        let mut processor = empty_processor();
        assert!(processor.process_new_rsk_event(&RskPegManagerEvents::IgnoredEvent).is_ok());
    }
}
