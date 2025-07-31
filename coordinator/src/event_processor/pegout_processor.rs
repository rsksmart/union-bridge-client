use crate::blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView};
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
    BtcSignatureSubFlowFactoryApi, SIGNATURE_MESSAGE,
};
use crate::{
    config::REQUIRED_CONFIRMATIONS,
    event_processor::EventProcessor,
    types::{EventWithBlock, RskPegManagerEvents},
};
use anyhow::{Context, Result, anyhow, bail};
use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, VariableTypes,
};
use common::runtime_sync::RuntimeSync;
use common::types::{Hash256, TxHash};
use common::{
    msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi},
    types::RskBlockAndUncles,
};
use log::{debug, info, trace};
use serde::Serialize;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{RegisterPegoutInput, RegisterPegoutOutput};
use union_contracts::bindings::peg_manager::PegManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

pub const USER_TAKE: &str = "USER_TAKE";
pub const PROGRAM_TYPE_REQUEST_PEGOUT: &str = "request_pegout";
pub const PEGOUT_REQUEST_NAME: &str = "pegout_request";

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

#[derive(Debug, Clone)]
struct PegoutEventState<BSF: BtcSignatureSubFlowApi> {
    pegout_requested_tx: TxHash,
    pegout_requested: PegoutEvent<PegoutRequested>,
    pegout_registered_tx: Option<TxHash>,
    pegout_registered: Option<PegoutEvent<PegoutRegistered>>,
    all_signatures_ready: Option<PegoutEvent<Hash256>>,
    btc_signatures_flow: Option<BSF>,
}

impl<BSF: BtcSignatureSubFlowApi> PegoutEventState<BSF> {
    fn new(pegout_requested_tx: TxHash, pegout_requested: PegoutEvent<PegoutRequested>) -> Self {
        Self {
            pegout_requested_tx,
            pegout_requested,
            pegout_registered_tx: None,
            pegout_registered: None,
            all_signatures_ready: None,
            btc_signatures_flow: None,
        }
    }
}

pub struct PegoutProcessor<CG, BC, BSF, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
{
    rt_sync: RuntimeSync,
    contracts_gateway: Rc<CG>,
    bitvmx_broker: Rc<BC>,
    blockchain: BlockchainView,
    tracker: HashMap<Uuid, PegoutEventState<BSF>>,
    btc_sig_subflow_factory: FactoryBSF,
}

impl<CG, BC>
    PegoutProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(rt_sync: RuntimeSync, contracts_gateway: Rc<CG>, bitvmx_broker: Rc<BC>) -> Self {
        Self {
            rt_sync: rt_sync.clone(),
            contracts_gateway: contracts_gateway.clone(),
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            tracker: HashMap::new(),
            btc_sig_subflow_factory: BtcSignatureSubFlowFactory::new(contracts_gateway, rt_sync),
        }
    }

    fn notify_pegout_requested_to_bitvmx(
        bitvmx_broker: &BC,
        flow_id: Uuid,
        event_data: &impl Serialize,
    ) -> Result<()> {
        //Set var must be sent first
        Self::send_set_var_to_bitvmx(bitvmx_broker, flow_id, PEGOUT_REQUEST_NAME, event_data)
            .context(format!(
                "Error processing confirmed pegout event (flow_id: {})",
                flow_id
            ))?;

        //Setup must be sent after set_var
        Self::send_setup_to_bitvmx(bitvmx_broker, flow_id)?;

        Ok(())
    }

    fn track_pegout_requested(&mut self, event: EventWithBlock<PegoutRequested>) -> Result<()> {
        // Check if exist any pegout_event_state into the tracker having pegout_requested_tx equal to event.tx_hash
        if self
            .tracker
            .values()
            .any(|state| state.pegout_requested_tx == event.tx_hash)
        {
            bail!(
                "Pegout request already exists for tx_hash: {}",
                event.tx_hash
            );
        }

        let flow_id = Uuid::new_v4();
        let tx_hash = event.tx_hash.clone();

        let observer_id = format!("pegout_requested-{}", flow_id);
        let confirmations =
            BlockConfirmations::new(observer_id, event.block_number, REQUIRED_CONFIRMATIONS);

        let pegout_requested_event = PegoutEvent::new(event.clone(), confirmations);

        self.blockchain
            .add_observer(pegout_requested_event.confirmations.clone());
        info!(
            "Adding PegoutRequested event to the pegout event tracker. id: {} Event{:?}",
            flow_id, pegout_requested_event
        );

        let pegout_event_state = PegoutEventState::new(tx_hash, pegout_requested_event);
        self.tracker.insert(flow_id, pegout_event_state);

        Ok(())
    }

    fn untrack_pegout_requested(&mut self, event: &EventWithBlock<PegoutRequested>) -> Result<()> {
        // Find the pegout event state for the given transaction hash
        let (flow_id, pegout_event) = self
            .tracker
            .iter()
            .find(|(_, value)| value.pegout_requested_tx == event.tx_hash)
            .ok_or_else(|| anyhow!("Pegout not found for tx_hash: {}", event.tx_hash))?;

        let flow_id = flow_id.clone();

        if pegout_event.pegout_registered.is_some() {
            bail!(
                "Pegout registered found while trying to remove PegoutRequested event {:?}=>{:?}",
                event,
                pegout_event.pegout_registered
            );
        }

        let observer_id = pegout_event
            .pegout_requested
            .confirmations
            .borrow()
            .get_id();
        self.blockchain.remove_observer(&observer_id);

        info!(
            "Untracked pegout requested event, pegout_requested_tx={:?}, flow_id={:?},",
            pegout_event.pegout_requested_tx, flow_id
        );

        self.tracker.remove(&flow_id);

        if self.tracker.is_empty() {
            debug!("Pegout tracker is empty, clearing blockchain observers");
            self.blockchain.clear();
        }
        Ok(())
    }

    fn _handle_register_pegout(
        &mut self,
        flow_id: &Uuid,
        input: RegisterPegoutInput,
    ) -> Result<RegisterPegoutOutput> {
        let pegout_event = self
            .tracker
            .get_mut(flow_id)
            .ok_or_else(|| anyhow!("Pegout not found for flow_id: {}", flow_id))?;
        if pegout_event.pegout_registered.is_some() {
            bail!("Pegout already registered for flow_id: {}", flow_id);
        }
        let result = self
            .rt_sync
            .run(async { self.contracts_gateway.register_pegout(input).await })?;

        pegout_event.pegout_registered_tx =
            Some(TxHash::try_from(result.transaction_hash.as_str())?);
        Ok(result)
    }

    fn track_pegout_registered(&mut self, event: EventWithBlock<PegoutRegistered>) -> Result<()> {
        let (flow_id, pegout_event_state) = self
            .tracker
            .iter_mut()
            .find(|(_, value)| {
                value
                    .pegout_registered_tx
                    .map_or(false, |tx| tx == event.tx_hash)
            })
            .ok_or_else(|| anyhow!("Pegout registered not found for tx_hash: {}", event.tx_hash))?;

        let observer_id = format!("pegout_registered-{}", flow_id);

        let confirmations =
            BlockConfirmations::new(observer_id, event.block_number, REQUIRED_CONFIRMATIONS);

        let pegout_event: PegoutEvent<PegoutRegistered> =
            PegoutEvent::new(event.clone(), confirmations);

        self.blockchain
            .add_observer(pegout_event.confirmations.clone());
        info!(
            "Adding PegoutRegistered event to the pegout event tracker. Id: {} Event{:?}",
            flow_id, pegout_event
        );

        pegout_event_state.pegout_registered = Some(pegout_event);
        Ok(())
    }

    fn untrack_pegout_registered(&mut self, event: EventWithBlock<PegoutRegistered>) -> Result<()> {
        // Find the pegout event state for the given transaction hash
        let flow_id = {
            let (flow_id, pegout_event) = self
                .tracker
                .iter_mut()
                .find(|(_, value)| {
                    value
                        .pegout_registered_tx
                        .map_or(false, |tx| tx == event.tx_hash)
                })
                .ok_or_else(|| anyhow!("Pegout not found for tx_hash: {}", event.tx_hash))?;

            // Validate that pegout registered event exists
            if pegout_event.pegout_registered.is_none() {
                bail!(
                    "Pegout registered not found while trying to remove PegoutRegistered event {:?}=>{:?}",
                    event,
                    pegout_event.pegout_registered
                );
            }

            // Remove the blockchain observer
            let observer_id = pegout_event
                .pegout_registered
                .as_ref()
                .ok_or_else(|| anyhow!("Pegout registered event not found"))?
                .confirmations
                .borrow()
                .get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            // Clear the pegout registered event
            pegout_event.pegout_registered = None;

            info!(
                "Untracked pegout registered event, pegout_registered_tx={:?}, flow_id={:?}",
                pegout_event.pegout_registered_tx, *flow_id
            );

            *flow_id
        };

        // Remove from tracker and clean up if empty
        self.tracker.remove(&flow_id);
        if self.tracker.is_empty() {
            debug!("Pegout tracker is empty, clearing blockchain observers");
            self.blockchain.clear();
        }

        Ok(())
    }

    fn handle_bitvmx_request(
        &mut self,
        _flow_id: &Uuid,
        method_name: &str,
        _json_value: &Value,
    ) -> Result<Value> {
        match method_name {
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

    //TOOD To update data struct:
    /*
       #[derive(Debug, Clone, Serialize, Deserialize)]
       pub struct PegOutRequest {
           pub committee_id: Uuid,
           pub slot_id: u32,
           pub fee: u64,
           pub user_pubkey: PublicKey,
           pub take_aggregated_key: PublicKey,
       }
    */
    fn send_set_var_to_bitvmx<E: Serialize>(
        bitvmx_broker: &BC,
        flow_id: Uuid,
        variable_name: &str,
        data: &E,
    ) -> Result<()> {
        let data = serde_json::to_string(data)?;
        debug!("Sending set_var with id: {} and data: {}", flow_id, data);
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
        debug!(
            "Sending dispatch transaction name msg to bitvmx with id: {}",
            flow_id
        );
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
            info!("Confirmed pegout requested id: {}", flow_id);
            Self::notify_pegout_requested_to_bitvmx(
                &self.bitvmx_broker,
                *flow_id,
                &event.data.inner,
            )?;

            event.mark_handled();

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            info!(
                "Successfully processed confirmed pegout requested event: {}",
                flow_id
            );
        }

        Ok(())
    }

    fn process_unhandled_confirmed_pegout_registered_events(&mut self) -> Result<()> {
        let mut flow_ids_to_remove: Vec<Uuid> = Vec::new();

        for (flow_id, event) in self.tracker.iter_mut().filter_map(|(flow_id, state)| {
            state
                .pegout_registered
                .as_mut()
                .filter(|event| event.is_confirmed() && !event.is_handled)
                .map(|event| (*flow_id, event))
        }) {
            Self::send_set_var_to_bitvmx(
                &self.bitvmx_broker,
                flow_id,
                "PEG_OUT_COMPLETED",
                &event.data.inner,
            )
            .context(format!(
                "Error processing confirmed pegout event (flow_id: {})",
                flow_id
            ))?;

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            flow_ids_to_remove.push(flow_id);

            info!(
                "Successfully processed confirmed pegout registered event: {}",
                flow_id
            );
        }

        for flow_id in flow_ids_to_remove {
            self.tracker.remove(&flow_id);
        }
        if self.tracker.is_empty() {
            debug!("Pegout tracker is empty, clearing blockchain observers");
            self.blockchain.clear();
        }
        Ok(())
    }

    fn process_unhandled_confirmed_sig_flow_events(
        &mut self,
        block: &RskBlockAndUncles,
    ) -> Result<()> {
        for (flow_id, state) in self.tracker.iter_mut() {
            if let Some(btc_flow) = state.btc_signatures_flow.as_mut() {
                btc_flow.delegate_block(block)?;
            }
        }
        Ok(())
    }
}

impl<CG, BC> EventProcessor
    for PegoutProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            //TODO pending to define this message to start the pegout register step
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => match spv_proof_opt {
                Some(spv_proof) => {
                    info!(
                        "Received BitVMX SPVProof for tx_id: {}, proof: {:?}",
                        tx_id, spv_proof
                    );
                }
                None => bail!(
                    "Received BitVMX SPVProof event for tx_id: {}, but no SPV proof was included.",
                    tx_id
                ),
            },
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), "register-pegout") =>
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
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), SIGNATURE_MESSAGE) =>
            {
                let state = self
                    .tracker
                    .get_mut(flow_id)
                    .ok_or_else(|| anyhow!("Pegout not found for flow_id: {}", flow_id))?;
                if state.btc_signatures_flow.is_some() {
                    bail!(
                        "BTC signatures flow already exists for flow_id: {}",
                        flow_id
                    );
                } else {
                    let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(*flow_id);
                    let result = btc_sig_subflow.delegate_bitvmx_event(event)?;
                    state.btc_signatures_flow = Some(btc_sig_subflow);
                    result
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        trace!("Processing new event: {:?}", event);
        match event {
            RskPegManagerEvents::PegoutRequested(data) => {
                if data.removed {
                    info!("Handling Pegout Requested removed event: {:?}", data);
                    self.untrack_pegout_requested(data)?;
                    return Ok(());
                }
                debug!("Handling Pegout Requested event {:?}", data);
                self.track_pegout_requested(data.clone())?;
            }
            RskPegManagerEvents::PegoutRegistered(data) => {
                debug!("Handling Pegout Registered event {:?}", data);
                //TODO ask about how to relate the pegout_requested event with the pegout_registered event
                if data.removed {
                    self.untrack_pegout_registered(data.clone())?;
                    return Ok(());
                }
                self.track_pegout_registered(data.clone())?;
            }
            RskPegManagerEvents::AllNoncesReady(data)
            | RskPegManagerEvents::AllSignaturesReady(data) => {
                debug!("Handling signature event {:?}", data);
                for (flow_id, state) in self.tracker.iter_mut() {
                    if let Some(btc_flow) = state.btc_signatures_flow.as_mut() {
                        btc_flow.delegate_rsk_event(*flow_id, event)?;
                        if btc_flow.is_done() {
                            //TODO do the part related  with execute the pegout.
                        }
                    }
                }
            }
            _ => (),
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.tracker.is_empty() {
            return Ok(());
        }
        self.blockchain.update(block.clone());

        self.process_unhandled_confirmed_pegout_requested_events()?;
        self.process_unhandled_confirmed_pegout_registered_events()?;
        self.process_unhandled_confirmed_sig_flow_events(block)?;
        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutProcessor");
        self.blockchain.clear();
        self.tracker.clear();
    }
}
