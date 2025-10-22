/*
U: Union Bridge
B: BitVMX Client
RSK: RSK Blockchain Gateway

Step 1: PegoutRequested event is received (RSK -> U)
    a: Send setVar
    b: Send setup
Step 2: PegoutAccepted event is received (B -> U)
    a: Register nonces (U -> RSK)
Step 3: AllNoncesReady event is received (RSK -> U)
    a: Register signatures (U -> RSK)
Step 4: AllSignaturesReady event is received (RSK -> U)
    a: Dispatch transaction name (U -> B)
    b: Ask for transaction status (U -> B)
Step 5: Transaction status is received (B -> U)
    a: If confirmations are not enough, schedule a new request for a newtransaction status
    b: If confirmations are enough, request SPV proof (U -> B)
Step 6: SPVProof is received (B -> U)
    a: Register pegout calling the peg manager contract (U -> C)
Step 7: Pegout Registered event is received (RSK -> U)
    a: Confirm pegout registered and sending the confirmation to BitVMX with SetVar
*/
use crate::blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView};
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
    BtcSignatureSubFlowFactoryApi,
};
use crate::flows::common::{COMM_KEY_INDEX, GlobalContext, build_communication_data};
use crate::types::{RegisterSignaturesBitVmxData, TickScheduler};
use crate::{
    config::REQUIRED_CONFIRMATIONS,
    event_processor::EventProcessor,
    types::{EventWithBlock, RskPegManagerEvents},
};
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::{PublicKey, Txid};
use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, P2PAddress, PeerId, PegOutAccepted,
    PegOutRequest, TransactionStatus, VariableTypes,
};
use common::runtime_sync::RuntimeSync;
use common::types::{CommitteeId, TxHash};
use common::{
    msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi},
    types::RskBlockAndUncles,
};
use log::{debug, info, trace};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{
    GetCommitteeInput, GetCommitteeOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
    P2PAddressParser,
};
use transaction_dispatcher::types::{RegisterPegoutInput, RegisterPegoutOutput};
use union_contracts::bindings::peg_manager::PegManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

pub const USER_TAKE_TX: &str = "USER_TAKE_TX";
pub const PROGRAM_TYPE_USER_TAKE: &str = "take";
pub const PEGOUT_ACCEPTED_NAME: &str = "pegout_accepted";
pub const BLOCKS_DELAY_FOR_TX_CHECK: u32 = 10; // Number of blocks to wait before rechecking transaction status
//TODO (JIRA) https://rsklabs.atlassian.net/browse/UB-328 pending to improve how these confirmations are handled
pub const SPV_PROOF_MIN_CONFIRMATIONS: u32 = 1 + 1; // +1 from Contracts, +1 to give time to the Native Bridge to get up to date with Bitcoin Node

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
    user_take_tx_id: Option<Txid>,
    pegout_registered_tx: Option<TxHash>,
    pegout_registered: Option<PegoutEvent<PegoutRegistered>>,
    btc_sig_flow: Option<BSF>,
}

impl<BSF: BtcSignatureSubFlowApi> PegoutEventState<BSF> {
    fn new(pegout_requested_tx: TxHash, pegout_requested: PegoutEvent<PegoutRequested>) -> Self {
        Self {
            pegout_requested_tx,
            pegout_requested,
            user_take_tx_id: None,
            pegout_registered_tx: None,
            pegout_registered: None,
            btc_sig_flow: None,
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
    scheduler: TickScheduler<Uuid>,
    my_p2p_address: Option<P2PAddress>,
    global_context: GlobalContext,
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
    pub fn new(
        rt_sync: RuntimeSync,
        contracts_gateway: Rc<CG>,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
    ) -> Self {
        Self {
            rt_sync: rt_sync.clone(),
            contracts_gateway: contracts_gateway.clone(),
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            tracker: HashMap::new(),
            btc_sig_subflow_factory: BtcSignatureSubFlowFactory::new(contracts_gateway, rt_sync),
            scheduler: TickScheduler::new(),
            my_p2p_address: None,
            global_context,
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

    fn handle_tick(&mut self) -> Result<()> {
        if self.scheduler.is_empty() {
            return Ok(());
        }
        let ready = self.scheduler.tick();
        for flow_id in ready {
            debug!(
                "Sending delayed get transaction info by name to bitvmx with id: {}",
                flow_id
            );
            let tx_id = {
                let state = self
                    .tracker
                    .get(&flow_id)
                    .ok_or_else(|| anyhow!("Pegout state not found for flow_id: {}", flow_id))?;
                state
                    .user_take_tx_id
                    .ok_or_else(|| anyhow!("User take tx id not found for flow_id: {}", flow_id))?
            };
            Self::send_get_transaction(&self.bitvmx_broker, flow_id, tx_id)?;
        }
        Ok(())
    }

    fn notify_pegout_requested_to_bitvmx(
        &mut self,
        flow_id: Uuid,
        pegout_requested: &PegoutRequested,
    ) -> Result<()> {
        debug!(
            "Notifying pegout requested to bitvmx with flow_id: {}",
            flow_id
        );
        let committee_id: CommitteeId = pegout_requested.committeeId.try_into()?;
        let committee_output = self.get_committee_output(committee_id.clone())?;

        let data_to_send: PegOutRequest =
            self.pegout_requested_to_bitvmx_request(pegout_requested, &committee_output)?;

        let committee_peer_ids = self.get_committee_peer_ids(committee_output)?;

        //Step 1a: Send setVar (U -> B)
        //Set var must be sent before the setup
        Self::send_set_var_to_bitvmx(
            &self.bitvmx_broker,
            flow_id,
            PegOutRequest::name().as_str(),
            &data_to_send,
        )
        .context(format!(
            "Error processing confirmed pegout event (flow_id: {})",
            flow_id
        ))?;

        let committee_addresses = self.get_committee_member_address(committee_id)?;
        //TODO(JIRA) https://rsklabs.atlassian.net/browse/UB-315 if there were a delay in the response from bitvmx the first time it is possible that the p2p address is not available yet, so we need to handle that case.
        // is expected to be modified in the refactor. The flow would wait for the get_comm_info_response to be received before continuing.
        let my_addr: &P2PAddress = self
            .my_p2p_address
            .as_ref()
            .ok_or_else(|| anyhow!("My P2P address not found"))?;

        let p2p_addresses = build_communication_data(
            my_addr.address.clone(),
            committee_addresses,
            committee_peer_ids,
        )?;
        //Step 1b: Setup BitVMX (U -> B)
        //Setup must be sent after set_var
        debug!("sending setup to bitvmx with id: {}", flow_id);
        Self::send_setup_to_bitvmx(&self.bitvmx_broker, flow_id, p2p_addresses)?;

        Ok(())
    }

    fn build_take_aggregated_key(committee_response: &GetCommitteeOutput) -> Result<PublicKey> {
        PublicKey::from_slice(&committee_response.committee.aggregatedKey)
            .context("Failed to parse aggregated public key from committee")
    }

    fn get_committee_member_address(&mut self, committee_id: CommitteeId) -> Result<Vec<String>> {
        let input = GetCommunicationDataInput {
            committee_id: committee_id.clone(),
            member_address: self.contracts_gateway.my_address().into(),
        };
        let member_comm_data = self.rt_sync.run(async {
            self.contracts_gateway
                .get_committee_communication_data(input)
                .await
        })?;

        let committee_addresses = member_comm_data
            .communication_data
            .into_iter()
            .map(|comm_data| {
                P2PAddressParser::addr_from_contracts(&comm_data)
                    .context("Failed to convert communication data to P2P address")
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(committee_addresses)
    }
    fn get_committee_output(&mut self, committee_id: CommitteeId) -> Result<GetCommitteeOutput> {
        let committee_response = self.rt_sync.run(async {
            self.contracts_gateway
                .get_committee(GetCommitteeInput { committee_id })
                .await
        })?;
        Ok(committee_response)
    }

    fn get_committee_peer_ids(
        &mut self,
        committee_output: GetCommitteeOutput,
    ) -> Result<Vec<PeerId>> {
        let mut peer_ids = Vec::new();

        for member in committee_output.committee.members {
            // Get the member's public keys
            let keys_input = GetMemberPublicKeysInput {
                member_address: member.memberAddress,
            };

            let keys_response = self.rt_sync.run(async {
                self.contracts_gateway
                    .get_member_public_keys(keys_input)
                    .await
            })?;

            // Get the communication key (at index 2)
            let key_str = keys_response
                .public_keys
                .get(COMM_KEY_INDEX)
                .context(format!(
                    "Communication key not found for member {}",
                    member.memberAddress
                ))?;

            debug!("Member {} PeerId: {:?}", member.memberAddress, key_str);
            peer_ids.push(PeerId(key_str.to_string()));
        }

        Ok(peer_ids)
    }

    fn pegout_requested_to_bitvmx_request(
        &mut self,
        event: &PegoutRequested,
        committee_output: &GetCommitteeOutput,
    ) -> Result<PegOutRequest> {
        debug!(
            "Preparing PegOutRequest for BitVMX from PegoutRequested event: {:?}",
            event
        );

        let committee_id: Uuid = Uuid::from_u128(event.committeeId.try_into()?);

        // Convert user pubkey bytes to bitcoin::PublicKey
        let user_pubkey = if event.userPubKey.len() == 33 {
            // Try parsing as compressed public key (33 bytes with prefix)
            debug!("Attempting to parse as compressed public key (33 bytes)");
            PublicKey::from_slice(event.userPubKey.as_ref())
                .context("Failed to parse user public key as compressed public key")?
        } else {
            bail!(
                "Invalid user public key length: {}, expected 33",
                event.userPubKey.len()
            );
        };

        let take_aggregated_key = Self::build_take_aggregated_key(&committee_output)?;

        // Convert fixed-size hashes and ids to Vec<u8>
        let pegout_signature_hash: Vec<u8> = event.pegoutSignatureHash.as_slice().to_vec();
        let pegout_id: Vec<u8> = event.pegoutId.as_slice().to_vec();
        let pegout_signature_message: Vec<u8> = event.pegoutSignatureMessage.clone().to_vec();
        let slot_index = event.slotId as usize;

        Ok(PegOutRequest {
            committee_id,
            stream_id: event.streamId,
            packet_number: event.packetNumber,
            slot_index,
            amount: event.amount,
            pegout_id,
            pegout_signature_hash,
            pegout_signature_message,
            user_pubkey,
            take_aggregated_key,
        })
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
        let committee_id: CommitteeId = event.inner.committeeId.try_into()?;

        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!(
                "Handling PegoutRequested event with committee id {}, I am NOT member so I skip",
                committee_id
            );
            return Ok(());
        }
        debug!(
            "Handling PegoutRequested event with committee id {}, as member I should respond",
            committee_id
        );

        let slot_index = event.inner.slotId as usize;
        let committee_uuid: Uuid = Uuid::from_u128(event.inner.committeeId.try_into()?);
        let flow_id = Self::get_user_take_pid(committee_uuid, slot_index)?;
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
            self.scheduler.clear();
        }
        Ok(())
    }

    fn handle_register_pegout_with_state(
        rt_sync: &RuntimeSync,
        contracts_gateway: &CG,
        state: &mut PegoutEventState<BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>>,
        tx_id: &Txid,
        input: RegisterPegoutInput,
    ) -> Result<RegisterPegoutOutput> {
        // Step 6a: Register pegout calling the peg manager contract (U -> C)
        match rt_sync.run(async { contracts_gateway.register_pegout(input).await }) {
            Ok(result) => {
                info!("Pegout registered successfully for tx_id: {}", tx_id);
                state.pegout_registered_tx =
                    Some(TxHash::try_from(result.transaction_hash.as_str())?);
                return Ok(result);
            }
            Err(e) => {
                bail!("Pegout registration failed for tx_id: {tx_id} - {e}");
            }
        }
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

    fn send_setup_to_bitvmx(
        bitvmx_broker: &BC,
        flow_id: Uuid,
        p2p_address: Vec<P2PAddress>,
    ) -> Result<()> {
        bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::Setup(
                flow_id,
                PROGRAM_TYPE_USER_TAKE.to_string(),
                p2p_address,
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

    fn send_get_comm_info_to_bitvmx(bitvmx_broker: &BC) -> Result<()> {
        bitvmx_broker.send(BROKER_SERVER_ID, IncomingBitVMXApiMessages::GetCommInfo())?;
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
            IncomingBitVMXApiMessages::DispatchTransactionName(flow_id, USER_TAKE_TX.to_string()),
        )?;
        Ok(())
    }

    fn send_get_transaction(bitvmx_broker: &BC, flow_id: Uuid, tx_id: Txid) -> Result<()> {
        let message = IncomingBitVMXApiMessages::GetTransaction(flow_id, tx_id);
        bitvmx_broker.send(BROKER_SERVER_ID, message)?;
        Ok(())
    }

    fn send_get_spv_proof_to_bitvmx(bitvmx_broker: &BC, tx_id: Txid) -> Result<()> {
        let msg = IncomingBitVMXApiMessages::GetSPVProof(tx_id);
        bitvmx_broker.send(BROKER_SERVER_ID, msg)?;
        Ok(())
    }

    fn handle_transaction_status_received(
        &mut self,
        flow_id: &Uuid,
        tx_status: TransactionStatus,
    ) -> Result<()> {
        //find the pegout event state for the given flow_id
        let state = self
            .tracker
            .get_mut(flow_id)
            .ok_or_else(|| anyhow!("Pegout state not found for flow_id: {}", flow_id))?;
        if state.user_take_tx_id != Some(tx_status.tx_id) {
            bail!(
                "Pegout state for flow_id: {} does not match tx_id: {}",
                flow_id,
                tx_status.tx_id
            );
        }
        if tx_status.confirmations >= SPV_PROOF_MIN_CONFIRMATIONS {
            debug!(
                "Transaction confirmed with sufficient confirmations for flow_id: {}",
                flow_id
            );
            if self.scheduler.is_scheduled(flow_id) {
                debug!(
                    "Unscheduling get transaction info by name to bitvmx with id: {}",
                    flow_id
                );
                self.scheduler.cancel(flow_id);
            }
            // Step 5b: If confirmations are enough, request SPV proof (U -> B)
            Self::send_get_spv_proof_to_bitvmx(&self.bitvmx_broker, tx_status.tx_id)?;
        } else {
            // Step 5a: If confirmations are not enough, schedule a new request for a newtransaction status
            debug!(
                "Transaction not confirmed with sufficient confirmations for flow_id: {}",
                flow_id
            );
            debug!(
                "Scheduling get transaction info by name to bitvmx with id: {}",
                flow_id
            );
            self.scheduler.schedule(*flow_id, BLOCKS_DELAY_FOR_TX_CHECK);
        }
        Ok(())
    }

    fn process_unhandled_confirmed_pegout_requested_events(&mut self) -> Result<()> {
        let mut vec_to_process: Vec<(Uuid, PegoutEvent<PegoutRequested>)> = Vec::new();
        for (flow_id, state) in self.tracker.iter_mut() {
            let event = &mut state.pegout_requested;
            if !event.is_confirmed() || event.is_handled {
                continue;
            }
            event.mark_handled();
            vec_to_process.push((*flow_id, event.clone()));
        }

        for (flow_id, event) in vec_to_process {
            // Step 1a: Notify BitVMX about the pegout request (U -> B)
            info!("Confirmed pegout requested id: {}", flow_id);
            self.notify_pegout_requested_to_bitvmx(flow_id, &event.data.inner)?;

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

    //Step 7a confirm pegout registered and sending the confirmation to BitVMX with SetVar
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
            if let Some(btc_flow) = state.btc_sig_flow.as_mut() {
                btc_flow.delegate_block(block)?;
                if btc_flow.is_done() {
                    // Step 4a: Dispatch transaction name (U -> B)
                    Self::send_dispatch_transaction_name_msg_to_bitvmx(
                        &self.bitvmx_broker,
                        *flow_id,
                    )?;
                    debug!(
                        "Pegout Dispatch tx sent. It is expected to receive Transaction Status from Bitvmx after Tx be mined"
                    );
                    state.btc_sig_flow = None;
                }
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
            OutgoingBitVMXApiMessages::CommInfo(p2p_address) => {
                debug!("Received CommInfo from BitVMX: {:?}", p2p_address);
                self.my_p2p_address = Some(p2p_address.clone());
            }
            // Step 6: SPVProof is received (B -> U)
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => match spv_proof_opt {
                Some(spv_proof) => {
                    info!(
                        "Received BitVMX SPVProof for tx_id: {}, proof: {:?}",
                        tx_id, spv_proof
                    );

                    // Find the matching state with this user_take_tx_id
                    let (_, state) = match self
                        .tracker
                        .iter_mut()
                        .find(|(_, state)| state.user_take_tx_id == Some(*tx_id))
                    {
                        Some(result) => result,
                        None => {
                            debug!(
                                "No pending user take transaction found for tx_id: {}",
                                tx_id
                            );
                            return Ok(());
                        }
                    };

                    let register_pegout_input: transaction_dispatcher::types::BtcTxSPVProofInput =
                        RegisterPegoutInput::from(spv_proof.clone());
                    Self::handle_register_pegout_with_state(
                        &self.rt_sync,
                        &self.contracts_gateway,
                        state,
                        tx_id,
                        register_pegout_input,
                    )?;
                }
                None => bail!(
                    "Received BitVMX SPVProof event for tx_id: {}, but no SPV proof was included.",
                    tx_id
                ),
            },
            // Step 2: PegoutAccepted event is received (B -> U)
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), PEGOUT_ACCEPTED_NAME) =>
            {
                let state = self
                    .tracker
                    .get_mut(flow_id)
                    .ok_or_else(|| anyhow!("Pegout not found for flow_id: {}", flow_id))?;
                if state.btc_sig_flow.is_some() {
                    bail!(
                        "BTC signatures flow already exists for flow_id: {}",
                        flow_id
                    );
                } else {
                    let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(*flow_id);
                    let input: PegOutAccepted = serde_json::from_str::<PegOutAccepted>(data)?;
                    state.user_take_tx_id = Some(input.user_take_txid);
                    let register_input = RegisterSignaturesBitVmxData::try_from(input)?;
                    // Step 2a: Register nonces (U -> RSK)
                    btc_sig_subflow.start_signature_flow(*flow_id, &register_input)?;
                    state.btc_sig_flow = Some(btc_sig_subflow);
                }
            }
            // Step 5: Transaction status is received (B -> U)
            OutgoingBitVMXApiMessages::Transaction(flow_id, tx_status, _tx_opt) => {
                debug!(
                    "Received BitVMX Transaction event. Flow Id: {}, Tx Status: {:?}",
                    flow_id, tx_status
                );
                self.handle_transaction_status_received(flow_id, tx_status.clone())?;
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        trace!("Processing new event: {:?}", event);
        match event {
            // Step 1: PegoutRequested event is received (RSK -> U)
            RskPegManagerEvents::PegoutRequested(data) => {
                if data.removed {
                    info!("Handling Pegout Requested removed event: {:?}", data);
                    self.untrack_pegout_requested(data)?;
                    return Ok(());
                }
                Self::send_get_comm_info_to_bitvmx(&self.bitvmx_broker)?;
                debug!("Handling Pegout Requested event {:?}", data);
                self.track_pegout_requested(data.clone())?;
            }
            // Step 7: Pegout Registered event is received (RSK -> U)
            RskPegManagerEvents::PegoutRegistered(data) => {
                debug!("Handling Pegout Registered event {:?}", data);
                if data.removed {
                    self.untrack_pegout_registered(data.clone())?;
                    return Ok(());
                }
                self.track_pegout_registered(data.clone())?;
            }
            // Step 3: AllNoncesReady event is received (RSK -> U)
            RskPegManagerEvents::AllNoncesReady(data)
            // Step 4: AllSignaturesReady event is received (RSK -> U)
            | RskPegManagerEvents::AllSignaturesReady(data) => {
                debug!("Handling signature event {:?}", data);

                for (flow_id, state) in self.tracker.iter_mut() {
                    if let Some(btc_flow) = state.btc_sig_flow.as_mut() {
                        // Step 3a: Register signatures (U -> RSK)
                        btc_flow.delegate_rsk_event(*flow_id, event)?;
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
        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.handle_tick()?;
        self.process_unhandled_confirmed_pegout_registered_events()?;
        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutProcessor");
        self.blockchain.clear();
        self.tracker.clear();
    }
}
