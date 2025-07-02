use crate::types::AllSignaturesReadyEvent;
use crate::{
    config::REQUIRED_CONFIRMATIONS,
    event_processor::{
        EventProcessor,
        blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView},
    },
    types::{EventWithBlock, RskPegManagerEvents},
};
use anyhow::bail;
use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, VariableTypes,
};
use common::msg_broker::broker::PROGRAM_TYPE_REQUEST_PEGOUT;
use common::runtime_sync::RuntimeSync;
use common::types::Hash256;
use common::{
    msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi, USER_TAKE},
    types::RskBlockAndUncles,
};
use log::{debug, info, trace};
use serde::Serialize;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{RegisterPegOutInput, RegisterPegOutOutput, TryPegOutInput};
use union_contracts::bindings::peg_manager::PegManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct PegoutEvent<T: Clone> {
    data: EventWithBlock<T>,
    confirmations: Rc<RefCell<BlockConfirmations>>,
    is_handled: bool,
}

impl<T: Clone> PegoutEvent<T> {
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
struct PegoutEventState {
    pegout_requested: PegoutEvent<PegoutRequested>,
    pegout_registered: Option<PegoutEvent<PegoutRegistered>>,
    pegout_registered_handled: bool,
    all_signatures_ready: Option<PegoutEvent<Hash256>>,
}

impl PegoutEventState {
    fn new(pegout_requested: PegoutEvent<PegoutRequested>) -> Self {
        Self {
            pegout_requested,
            pegout_registered: None,
            pegout_registered_handled: false,
            all_signatures_ready: None,
        }
    }
}

pub struct PegoutProcessor<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi> {
    rt_sync: RuntimeSync,
    contracts_gateway: Arc<CG>,
    bitvmx_broker: Arc<BC>,
    blockchain: BlockchainView,
    tracker: HashMap<Uuid, PegoutEventState>,
}

impl<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi> PegoutProcessor<CG, BC> {
    pub fn new(rt_sync: RuntimeSync, contracts_gateway: Arc<CG>, bitvmx_broker: Arc<BC>) -> Self {
        Self {
            rt_sync,
            contracts_gateway,
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            tracker: HashMap::new(),
        }
    }

    fn handle_register_pegout(
        &mut self,
        flow_id: &Uuid,
        input: RegisterPegOutInput,
    ) -> Result<RegisterPegOutOutput> {
        let pegout_event = self
            .tracker
            .get_mut(flow_id)
            .ok_or_else(|| anyhow::anyhow!("Pegout not found for flow_id: {}", flow_id))?;
        if pegout_event.pegout_registered_handled {
            bail!("Pegout already registered for flow_id: {}", flow_id);
        }
        let result = self
            .rt_sync
            .run(async { self.contracts_gateway.register_peg_out_request(input).await })?;
        pegout_event.pegout_registered_handled = true;
        Ok(result)
    }

    fn handle_bitvmx_request(
        &mut self,
        flow_id: &Uuid,
        method_name: &str,
        json_value: &Value,
    ) -> Result<Value> {
        match method_name {
            "pegout-request" => {
                let input: TryPegOutInput = serde_json::from_value(json_value.clone())?;
                let result = self
                    .rt_sync
                    .run(async { self.contracts_gateway.try_peg_out_request(input).await })?;
                Ok(serde_json::to_value(result)?)
                //TODO pending to dice if save or not requested pegouts
                //WE CAN SAVE A
                //Should we notify pegout requested?
            }
            "register-pegout" => {
                let input: RegisterPegOutInput = serde_json::from_value(json_value.clone())?;
                let result = self.handle_register_pegout(flow_id, input)?;
                Ok(serde_json::to_value(result)?)
            }
            "add-member-signature" => {
                info!("Add member signature received. To be implemented.");
                Ok(serde_json::json!({"status": "signature_added"}))
            }
            "add-member-nonce" => {
                info!("Add member nonce received. To be implemented.");
                Ok(serde_json::json!({"status": "nonce_added"}))
            }
            _ => bail!(
                "Unsupported method name for BitVMX response: {}",
                method_name
            ),
        }
    }

    fn track_all_signatures_ready(&mut self, event: AllSignaturesReadyEvent) -> Result<()> {
        //find into tracker values the one with event.inner.pegoutSignatureHash == pegout_signature_hash

        let (flow_id, pegout_event) = self
            .tracker
            .iter_mut()
            .find(|(_, value)| {
                value.all_signatures_ready.is_some()
                    && value.all_signatures_ready.as_ref().unwrap().data.inner == event.inner
            })
            .ok_or_else(|| {
                anyhow::anyhow!("Pegout not found for pegoutSignatureHash: {}", event.inner)
            })?;

        let observer_id = format!("all_signatures_ready-{}", flow_id);
        let confirmations =
            BlockConfirmations::new(observer_id, event.block_number, REQUIRED_CONFIRMATIONS);

        // Create AllSignaturesReady event from the hash
        let all_signatures_ready_event = PegoutEvent::new(event, confirmations);
        pegout_event.all_signatures_ready = Some(all_signatures_ready_event);
        Ok(())
    }

    fn untrack_all_signatures_ready(&mut self, event: AllSignaturesReadyEvent) -> Result<()> {
        let (flow_id, pegout_event) = self
            .tracker
            .iter_mut()
            .find(|(_, value)| {
                value.all_signatures_ready.is_some()
                    && value.all_signatures_ready.as_ref().unwrap().data.inner == event.inner
            })
            .ok_or_else(|| {
                anyhow::anyhow!("Pegout not found for pegoutSignatureHash: {}", event.inner)
            })?;

        let observer_id = {
            let confirmations = pegout_event
                .all_signatures_ready
                .as_ref()
                .unwrap()
                .confirmations
                .borrow();
            confirmations.get_id()
        };
        self.blockchain.remove_observer(observer_id.as_str());

        pegout_event.all_signatures_ready = None;

        let flow_id_owned: Uuid = *flow_id;
        info!(
            "Untracked all signatures ready event, flow_id={:?},",
            flow_id_owned
        );
        Ok(())
    }

    fn track_pegout_requested(
        &mut self,
        flow_id: Uuid,
        event: EventWithBlock<PegoutRequested>,
    ) -> Result<()> {
        if self.tracker.contains_key(&flow_id) {
            bail!("Pegout already registered for {}", flow_id);
        }
        let observer_id = format!("pegout_requested-{}", flow_id);
        let confirmations =
            BlockConfirmations::new(observer_id, event.block_number, REQUIRED_CONFIRMATIONS);

        let pegout_requested_event = PegoutEvent::new(event.clone(), confirmations);
        self.blockchain
            .add_observer(pegout_requested_event.confirmations.clone());
        info!(
            "Adding PegoutRequested event to the pegout event tracker. Event{:?}",
            pegout_requested_event
        );
        self.tracker
            .insert(flow_id, PegoutEventState::new(pegout_requested_event));
        Ok(())
    }

    fn track_pegout_registered(
        &mut self,
        flow_id: Uuid,
        event: EventWithBlock<PegoutRegistered>,
    ) -> Result<()> {
        let invented_uuid = Uuid::new_v4(); //TOOD we need to find a way to relate both request and register events
        let pegout_event_state = self
            .tracker
            .get_mut(&invented_uuid)
            .ok_or_else(|| anyhow::anyhow!("Pegout not found for flow_id: {}", invented_uuid))?;

        let observer_id = format!("pegout_registered-{}", flow_id);

        let confirmations =
            BlockConfirmations::new(observer_id, event.block_number, REQUIRED_CONFIRMATIONS);

        let pegout_event: PegoutEvent<PegoutRegistered> =
            PegoutEvent::new(event.clone(), confirmations);

        self.blockchain
            .add_observer(pegout_event.confirmations.clone());
        info!(
            "Adding PegoutRegistered event to the pegout event tracker. Event{:?}",
            pegout_event
        );

        pegout_event_state.pegout_registered = Some(pegout_event);
        Ok(())
    }

    fn untrack_pegout_requested(&mut self, event: EventWithBlock<PegoutRequested>) -> Result<()> {
        let pegout_signature_hash = event.inner.pegoutSignatureHash;
        //find in tracer value the one with same pegout_signature_hash
        let (flow_id, pegout_event) = self
            .tracker
            .iter_mut()
            .find(|(_, value)| {
                value.pegout_requested.data.inner.pegoutSignatureHash == pegout_signature_hash
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Pegout not found for pegout_signature_hash: {}",
                    pegout_signature_hash
                )
            })?;

        let flow_id_owned: Uuid = *flow_id;

        if pegout_event.pegout_registered.is_some() {
            bail!(
                "Pegout registered found while trying to remove PegoutRequested event {:?}=>{:?}",
                event,
                pegout_event.pegout_registered
            );
        }

        let observer_id = {
            let confirmations = pegout_event.pegout_requested.confirmations.borrow();
            confirmations.get_id()
        };

        self.blockchain.remove_observer(observer_id.as_str());
        info!(
            "Untracked pegout requested event, pegoutSignatureHash={:?}, flow_id={:?},",
            pegout_signature_hash, flow_id_owned
        );
        self.tracker.remove(&flow_id_owned);
        Ok(())
    }

    fn untrack_pegout_registered(&mut self, event: EventWithBlock<PegoutRegistered>) -> Result<()> {
        let invented_uuid = Uuid::new_v4(); //TOOD we need to find a way to relate both request and register events
        let pegout_event = self
            .tracker
            .get_mut(&invented_uuid)
            .ok_or_else(|| anyhow::anyhow!("Pegout not found for flow_id: {}", invented_uuid))?;
        if pegout_event.pegout_registered.is_none() {
            bail!(
                "Pegout registered notfound while trying to remove PegoutRegistered event {:?}=>{:?}",
                event,
                pegout_event.pegout_registered
            );
        }
        let observer_id = {
            let confirmations = pegout_event.pegout_requested.confirmations.borrow();
            confirmations.get_id()
        };
        self.blockchain.remove_observer(observer_id.as_str());
        pegout_event.pegout_registered = None;
        Ok(())
    }
    //TODO define with FG about this parameters
    fn send_setup_to_bitvmx(bitvmx_broker: &BC, flow_id: Uuid) -> Result<()> {
        bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::Setup(
                flow_id,
                PROGRAM_TYPE_REQUEST_PEGOUT.to_string(),
                vec![],
                0,
            ),
        )?;
        Ok(())
    }

    fn send_set_var_to_bitvmx<E: Serialize>(
        bitvmx_broker: &BC,
        flow_id: Uuid,
        variable_name: &str,
        data: &E,
    ) -> Result<()> {
        let data = serde_json::to_string(data)?;

        bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SetVar(
                flow_id,
                variable_name.to_string(),
                VariableTypes::String(data),
            ),
        )?;

        Ok(())
    }

    fn send_dispatch_transaction_name_msg_to_bitvmx(
        bitvmx_broker: &BC,
        flow_id: Uuid,
    ) -> Result<()> {
        bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::DispatchTransactionName(flow_id, USER_TAKE.to_string()),
        )?;
        Ok(())
    }

    fn process_unhandled_confirmed_pegout_requested_events(&mut self) -> Result<()> {
        for (flow_id, state) in self.tracker.iter_mut() {
            let event = &mut state.pegout_requested;
            if !event.is_confirmed() || event.is_handled {
                continue;
            }
            Self::send_setup_to_bitvmx(&self.bitvmx_broker, *flow_id)?;

            Self::send_set_var_to_bitvmx(
                &self.bitvmx_broker,
                *flow_id,
                "PegoutRequested",
                &event.data.inner,
            )
            .context(format!(
                "Error processing confirmed pegout event (flow_id: {})",
                flow_id
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

    fn process_unhandled_confirmed_pegout_registered_events(&mut self) -> Result<()> {
        let mut flow_id_to_remove: Option<Uuid> = None;

        for (flow_id, state) in self.tracker.iter_mut() {
            let event = match &mut state.pegout_registered {
                Some(event) => event,
                None => continue,
            };

            if !event.is_confirmed() || event.is_handled {
                continue;
            }

            Self::send_set_var_to_bitvmx(
                &self.bitvmx_broker,
                *flow_id,
                "PEGOUT_COMPLETED",
                &event.data.inner,
            )
            .context(format!(
                "Error processing confirmed pegout event (flow_id: {})",
                flow_id
            ))?;

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            flow_id_to_remove = Some(*flow_id);

            info!(
                "Successfully processed confirmed PeginRegistered event: {}",
                flow_id
            );
        }

        if let Some(flow_id) = flow_id_to_remove {
            self.tracker.remove(&flow_id);
        }

        Ok(())
    }

    fn process_unhandled_confirmed_all_signatures_ready_events(&mut self) -> Result<()> {
        for (flow_id, state) in self.tracker.iter_mut() {
            let event = match &mut state.all_signatures_ready {
                Some(event) => event,
                None => continue,
            };
            if !event.is_confirmed() || event.is_handled {
                continue;
            }

            Self::send_dispatch_transaction_name_msg_to_bitvmx(&self.bitvmx_broker, *flow_id)?;

            event.mark_handled();

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            info!(
                "Successfully processed confirmed AllSignaturesReady event: {}",
                flow_id
            );
        }

        Ok(())
    }
}

impl<CG: RskContractsGatewayApi, T: BitVmxBrokerClientApi> EventProcessor
    for PegoutProcessor<CG, T>
{
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), "request-pegout" | "register-pegout") =>
            {
                info!(
                    "Handling BitVMX Variable Event. Flow Id: {}, Method: {}, Payload: {:?}",
                    flow_id, method, data
                );

                let json_data = serde_json::from_str(data)?;
                let result = self.handle_bitvmx_request(flow_id, method, &json_data)?;

                info!(
                    "Successfully proxied request. Flow Id: {}, Method: '{}', Response: {}",
                    flow_id, method, result
                );
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> anyhow::Result<()> {
        trace!("Processing new event: {:?}", event);
        match event {
            RskPegManagerEvents::PegoutRequested(data) => {
                if data.removed {
                    info!("Handling PeginRequested removed event: {:?}", data);
                    self.untrack_pegout_requested(data.clone())?;
                    return Ok(());
                }
                debug!("Handling Pegout Requested event {:?}", data);
                let flow_id: Uuid = Uuid::new_v4();
                self.track_pegout_requested(flow_id, data.clone())?;
            }
            RskPegManagerEvents::PegoutRegistered(data) => {
                debug!("Handling Pegout Registered event {:?}", data);
                //TOOD ask about how to relate the pegout_requested event with the pegout_registered event
                //missing untrack pegout_requested event
                let invented_uuid = Uuid::new_v4();
                if data.removed {
                    self.untrack_pegout_registered(data.clone())?;
                    return Ok(());
                }
                self.track_pegout_registered(invented_uuid, data.clone())?;
            }
            RskPegManagerEvents::AllSignaturesReady(data) => {
                debug!("Handling AllSignaturesReady event {:?}", data);
                if data.removed {
                    self.untrack_all_signatures_ready(data.clone())?;
                    return Ok(());
                }
                self.track_all_signatures_ready(data.clone())?;
            }
            _ => (),
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> anyhow::Result<()> {
        if self.tracker.is_empty() {
            return Ok(());
        }
        self.blockchain.update(block.clone());

        self.process_unhandled_confirmed_pegout_requested_events()?;
        self.process_unhandled_confirmed_pegout_registered_events()?;
        self.process_unhandled_confirmed_all_signatures_ready_events()?;
        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutProcessor");
    }
}
