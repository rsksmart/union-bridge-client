use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::config::REQUIRED_CONFIRMATIONS;
use crate::event_processor::EventProcessor;
use crate::flows::common::GlobalContext;
use crate::flows::pegout::pegout_flow::Steps;
use crate::flows::pegout::pegout_flow::{PegoutFlow, StepData};
use crate::types::{EventStatus, RskPegManagerEvents, UserRequests};
use anyhow::anyhow;
use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::OutgoingBitVMXApiMessages;
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::{BlockNumber, CommitteeId, RskBlockAndUncles};
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use std::any::type_name_of_val;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use union_contracts::bindings::peg_manager::PegManager::PegoutRequested;
use uuid::Uuid;
/// Processor that manages multiple pegout flow state machines
pub struct PegoutFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    flows: HashMap<Uuid, PegoutFlow<CG, BC>>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
}

impl<CG, BC> PegoutFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            flows: HashMap::new(),
            global_context,
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
        }
    }

    pub fn get_user_take_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update(&slot_index.to_be_bytes());
        hasher.update("user_take");

        // Get the result as a byte array
        let hash = hasher.finalize();
        let slice = hash
            .as_slice()
            .get(..16)
            .ok_or_else(|| anyhow!("SHA256 hash too short for UUID generation"))?;
        let uuid_bytes: [u8; 16] = slice
            .try_into()
            .context("Failed to convert hash slice to UUID bytes")?;
        Ok(Uuid::from_bytes(uuid_bytes))
    }

    /// Create a new flow for a PegoutRequested event
    pub fn create_flow_for_pegout_requested(&mut self, event: &PegoutRequested) -> Result<()> {
        let committee_id: CommitteeId = event.committeeId.try_into()?;

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PegoutRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PegoutRequested event with committee id {}, as member I should respond",
            committee_id
        );

        let slot_index = event.slotId as usize;
        let committee_uuid: Uuid = Uuid::from_u128(event.committeeId.try_into()?);
        let flow_id = Self::get_user_take_pid(committee_uuid, slot_index)?;

        let mut flow = PegoutFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            self.global_context.clone(),
            flow_id,
            event.clone(),
        );

        // Initialize the flow with the PegoutRequested event
        flow.complete_step(StepData::PegoutRequested(event.clone()))?;

        self.flows.insert(flow_id, flow);

        info!(
            "Created new pegout flow {} for committee {}",
            flow_id, committee_id
        );
        Ok(())
    }

    /// Get the number of active flows
    pub fn active_flows_count(&self) -> usize {
        self.flows.len()
    }

    /// Clean up completed flows
    pub fn cleanup_completed_flows(&mut self) {
        let completed: Vec<_> = self
            .flows
            .iter()
            .filter(|(_, flow)| flow.is_done())
            .map(|(k, _)| *k)
            .collect();

        for internal_id in completed {
            debug!("Removing completed flow: {internal_id}");
            self.flows.remove(&internal_id);
        }
    }

    /// Process confirmed RSK events
    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        info!("Processing confirmed RSK event: {:?}", event);

        match event {
            RskPegManagerEvents::PegoutRequested(pr) => {
                info!("Processing confirmed PegoutRequested event: {:?}", pr);
                self.create_flow_for_pegout_requested(&pr.inner)?;
            }
            _ => {
                trace!("Ignoring confirmed RSK event: {}", type_name_of_val(event));
            }
        }

        self.cleanup_completed_flows();
        Ok(())
    }

    /// Build event info for PegoutRequested events
    fn build_pegout_requested_event_info(
        event: &crate::types::EventWithBlock<PegoutRequested>,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegout-requested-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PegoutRequested(event.clone()),
        )
    }
}

impl<CG, BC> EventProcessor for PegoutFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn process_user_request(&mut self, _req: &UserRequests) -> Result<()> {
        // Pegout flows are created from RSK events, not from user requests
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        trace!("Processing BitVMX event: {:?}", event);

        // TODO: Implement BitVMX event handling
        // For now, just log the events
        match event {
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                info!("Received CommInfo from BitVMX");
                //for any flow in flows having active step GetCommInfo, complete the step with the CommInfo
                for (flow_id, flow) in self.flows.iter_mut() {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Completing GetCommInfo step for flow {flow_id}");
                        flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            _ => {
                trace!("Ignoring BitVMX event: {:?}", event);
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        // useful for testing purposes
        if REQUIRED_CONFIRMATIONS == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::PegoutRequested(e) => Self::build_pegout_requested_event_info(e),
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if is_removal {
            warn!("Removing pending RSK event: {:?}", event);

            // properly clean up the observer before removing the event
            if let Some(mut removed_ev) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_ev.stop_confirming() {
                    error!("Failed to stop confirming for removed event {id}: {e}")
                }
            } else {
                warn!("Tried to remove non-existing pending event with id {id}");
            }
        } else {
            debug!("Adding new pending {event:?}, start confirming at block {block_num}");

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                REQUIRED_CONFIRMATIONS,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event
                .start_confirming(block_num)
                .context("Starting confirming")?;

            self.events_confirming
                .insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting for confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            trace!("No events left to confirm, skipping block");
            return Ok(());
        }

        self.blockchain_view.update(block.clone());

        // process confirmed events while removing them from the hashmap
        // collect the keys of confirmed events first to avoid mutating while iterating
        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter_map(|(key, event)| event.is_confirmed().then(|| key.clone()))
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                // properly cleanup the observer before processing the event
                if let Err(e) = event.stop_confirming() {
                    error!("Failed to stop confirming for event {}: {}", key, e)
                }
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        if self.events_confirming.is_empty() {
            debug!("No events left to confirm, clearing blockchain view");
            self.blockchain_view.clear();
        }

        // blocks allow periodic cleanup of completed flows, we can improve it with a cleanup task if needed
        self.cleanup_completed_flows();

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutFlowProcessor");
        self.flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
    }
}
