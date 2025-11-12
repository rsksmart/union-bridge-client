#![allow(unused)]

use crate::flows::common::{COMM_KEY_INDEX, GlobalContext, build_communication_data};
use crate::types::Role::Prover;
use crate::types::{RegisterSignaturesBitVmxData, TickScheduler};
use crate::{
    blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView},
    config::REQUIRED_CONFIRMATIONS,
    event_processor::EventProcessor,
    flows::btc_signature::btc_signature_subflow::{
        BtcSignatureSubFlowApi, BtcSignatureSubFlowFactoryApi,
    },
    types::{
        AllOperatorTakeTxHashesAddedEvent, EventWithBlock, PeginAcceptedEvent, PeginRequestedEvent,
        RskPegManagerEvents,
    },
};
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::{
    PublicKey, Txid,
    secp256k1::{Parity::Even, XOnlyPublicKey},
};
use common::msg_broker::bitvmx_types::{
    ACCEPT_PEGIN_TX, P2PAddress, PeerId, PeginAcceptedMessage, TransactionStatus,
};
use common::types::{CommitteeId, TxIdParser};
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
use log::{debug, error, info, trace, warn};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Debug,
    future::Future,
    rc::Rc,
};
use transaction_dispatcher::{
    rsk_gateway::{DomainErrors, RskContractsGatewayApi},
    types::{
        AddOperatorTakeTxHashInput, GetCommitteeInput, GetCommitteeOutput,
        GetCommunicationDataInput, GetMemberPublicKeysInput, P2PAddressParser, RequestPeginInput,
    },
};
use union_contracts::bindings::peg_manager::PegManager::{PeginAccepted, PeginRequested};
use union_contracts::bindings::signature_manager::SignatureManager::AllOperatorTakeTxHashesAdded;
use uuid::Uuid;

/**
U: Union Bridge
B: BitVMX
RSK: RSK Blockchain Gateway

Pegin steps:
1. Subscribe to BitVMX pegin events (U -> B)
2. Handle BitVMX PeginTransactionFound message received (B -> U)
    a. Ask BitVMX for SPV proof (do we need to check if the transaction is confirmed?) (U -> B)
3. Handle SPV proof received Request pegin step. (B -> U)
    a. Invoke PegManager requestPegin. (U -> RSK)
4. Handle PeguinRequested event from RSK (wait x confirmations) (RSK -> U)
    a. Send PeginRequestMessage to BitVMX through SetVar (U -> B)
    b. Send Setup to BitVMX //wait to setupCompleted?? (U -> B)
        i. It is needed to obtain p2p address of the committee members to be sent into the setup message.
           This info is being obtained from the SignatureManager.(U -> RSK)
5. Receive PeginAccepted message (B -> U)
    a Invoke AddOperatorTakeTxHash by calling SignatureManager (U -> RSK)
6. Handle RSK Event AllOperatorTakeTxHashesAdded (RSK -> U)
    a. AddMemberNonce by calling SignatureManager (U -> RSK)
7. AllNoncesReady RSK Event  (RSK -> U)
    a. AddMemberSignature by calling SignatureManager (U -> RSK)
8. AllSignaturesReady RSK Event (RSK -> U)
    a. Send DispatchTransaction to BitVMX (U -> B)
9. Receive Transaction message containing the transaction status (B -> U)
    a. If tx is not Confirmed, wait and ask for it again. (U -> B)
    b. If tx is Confirmed ask for the SPV proof (U -> B)
10. Handle SPV proof received Request pegin step. (B -> U)
    a. Invoke PegManager acceptPegin. (U -> RSK)
11. Handle PeginAccepted event from RSK (wait x confirmations) (RSK -> U)
    b. Send SetVar to BitVMX with the PEG_IN_COMPLETED msg (U -> B)

 */

const ACCEPT_PEGIN: &'static str = "accept-pegin";
const PEGIN_REQUEST: &'static str = "pegin_request";
const PEGIN_ACCEPTED_INPUT_MSG: &'static str = "pegin_accepted";
const PROGRAM_TYPE_ACCEPT_PEGIN: &'static str = "accept_pegin";
// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-328
pub const MIN_TX_CONFIRMATIONS: u32 = 1 + 1; // +1 from Contracts, +1 to give time to the Native Bridge to get up to date with Bitcoin Node
pub const BLOCKS_DELAY_FOR_TX_CHECK: u32 = 10;

/// Data structure used to send pegin request information to the BitVMX client.
/// This transforms raw blockchain events into a structured format with all necessary
/// committee and signature data that BitVMX needs for pegin processing.
#[derive(Debug, Clone, Serialize)]
struct PeginRequestMessage {
    txid: Txid, // requestPeginTxHash
    amount: u64,
    accept_pegin_sighash: Vec<u8>, // acceptPeginSignatureMessage
    take_aggregated_key: PublicKey,
    operator_indexes: Vec<usize>,
    slot_index: u64,
    committee_id: Uuid,
    rootstock_address: String,
    reimbursement_pubkey: PublicKey,
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
    bitvmx_pegin_accepted: Option<PeginAcceptedMessage>,
    btc_signatures_flow: Option<BSF>,
    all_operators_take_tx_hashes_added: Option<PeginEvent<AllOperatorTakeTxHashesAdded>>,
    my_p2p_address: Option<P2PAddress>,
}

impl<BSF: BtcSignatureSubFlowApi> PeginState<BSF> {
    fn new(pegin_flow_id: Uuid, pegin_requested: PeginEvent<PeginRequested>) -> Self {
        Self {
            flow_id: pegin_flow_id,
            pegin_requested,
            pegin_accepted: None,
            bitvmx_pegin_accepted: None,
            btc_signatures_flow: None,
            all_operators_take_tx_hashes_added: None,
            my_p2p_address: None,
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
    unconfirmed_pegin_requests: HashMap<String, BtcTxSPVProof>,
    unconfirmed_pegin_accepts: HashMap<Uuid, BtcTxSPVProof>,
    tracker: HashMap<TxHash, PeginState<BSF>>,
    pegin_request_tracker: HashSet<Txid>,
    btc_sig_subflow_factory: FactoryBSF,
    scheduler: TickScheduler<ScheduledAction>,
    global_context: GlobalContext,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum ScheduledAction {
    PeginRequested(String, i16), // Bitcoin block hash, attempt
    PeginAccepted(Uuid),
    PeginAcceptRetry(Uuid, i16), // flow_id, attempt
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
        global_context: GlobalContext,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        // Step 1: Subscribe to BitVMX pegin events
        Self::subscribe_to_bitvmx_pegin_events(&bitvmx_broker)
            .expect("Failed to subscribe to BitVMX pegin events");

        info!("Successfully subscribed to BitVMX pegin events");

        Self {
            rt_sync,
            contracts,
            bitvmx_broker,
            blockchain: BlockchainView::new(),
            unconfirmed_pegin_requests: HashMap::new(),
            unconfirmed_pegin_accepts: HashMap::new(),
            tracker: HashMap::new(),
            pegin_request_tracker: HashSet::new(),
            btc_sig_subflow_factory: factory,
            scheduler: TickScheduler::new(),
            global_context,
            native_bridge_verifier,
        }
    }

    fn tick_scheduler(&mut self) -> Result<()> {
        if self.scheduler.is_empty() {
            return Ok(());
        }

        let ready = self.scheduler.tick();

        for action in ready {
            trace!("Processing scheduled action: {:?}", action);

            match action {
                ScheduledAction::PeginAccepted(flow_id) => {
                    // Find the state with the matching flow_id
                    let state = self
                        .tracker
                        .values()
                        .find(|s| s.flow_id == flow_id)
                        .ok_or_else(|| anyhow!("State not found for flow_id: {}", flow_id))?;

                    // Get the accept_pegin_txid from bitvmx_pegin_accepted
                    let pegin_accepted = state.bitvmx_pegin_accepted.as_ref().ok_or_else(|| {
                        anyhow!("bitvmx_pegin_accepted is None for flow_id: {}", flow_id)
                    })?;

                    let accept_pegin_tx_hash = pegin_accepted.accept_pegin_txid;

                    debug!(
                        "Requesting transaction: acceptPeginTxHash={}, flow_id={}",
                        accept_pegin_tx_hash, flow_id
                    );

                    Self::send_get_transaction(&self.bitvmx_broker, flow_id, accept_pegin_tx_hash)?;
                }
                ScheduledAction::PeginRequested(btc_block, attempt) => {
                    debug!(
                        "(Re)trying handle_bitcoin_request_pegin for block {}, attempt={}",
                        btc_block, attempt
                    );

                    let Some(spv_proof) = self.unconfirmed_pegin_requests.get(&btc_block).cloned()
                    else {
                        info!(
                            "Skipping retry for block {}: no pending SPV proof found",
                            btc_block
                        );
                        continue;
                    };

                    self.send_request_pegin_contracts(
                        spv_proof,
                        attempt,
                        None,
                        Some(btc_block.clone()),
                    )?;
                }
                ScheduledAction::PeginAcceptRetry(flow_id, attempt) => {
                    debug!(
                        "(Re)trying accept_pegin for flow_id={}, attempt={}",
                        flow_id, attempt
                    );

                    let Some(spv_proof) = self.unconfirmed_pegin_accepts.get(&flow_id).cloned()
                    else {
                        info!(
                            "Skipping retry for flow_id={}: no pending SPV proof found",
                            flow_id
                        );
                        continue;
                    };

                    self.send_accept_pegin_contracts(spv_proof, flow_id, attempt)?;
                }
            }
        }
        Ok(())
    }

    pub fn get_accept_pegin_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update(&slot_index.to_be_bytes());
        hasher.update("accept_pegin");

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

    fn handle_pegin_requested(&mut self, data: &PeginRequestedEvent) -> Result<()> {
        if data.removed {
            debug!(
                "Removing PeginRequested: requestPeginTxHash={}, acceptPeginTxHash={}",
                data.inner.requestPeginTxHash, data.inner.acceptPeginTxHash
            );
            return self.untrack_pegin_requested(data.clone());
        }

        let committee_id = &data.inner.committeeId.try_into()?;
        if !self.global_context.my_committees().im_member(&committee_id) {
            trace!(
                "Skipping PeginRequested: not_committee_member, requestPeginTxHash={}",
                data.inner.requestPeginTxHash
            );
            return Ok(());
        }
        info!(
            "Processing PeginRequested: requestPeginTxHash={}, acceptPeginTxHash={}",
            data.inner.requestPeginTxHash, data.inner.acceptPeginTxHash
        );

        let committee_id = Uuid::from_u128(**committee_id);
        let slot_index = data.inner.streamPosition.slotId as usize;

        let pegin_flow_id = Self::get_accept_pegin_pid(committee_id, slot_index)?;
        let observer_id = format!("pegin_requested-{}", pegin_flow_id);
        let confirmations =
            BlockConfirmations::new(observer_id, data.block_number, REQUIRED_CONFIRMATIONS);
        let pegin_requested = PeginEvent::new(data.clone(), confirmations);

        self.blockchain
            .add_observer(pegin_requested.confirmations.clone());

        debug!(
            "Tracking PeginRequested: flow_id={}, requestPeginTxHash={}, acceptPeginTxHash={}",
            pegin_flow_id, data.inner.requestPeginTxHash, data.inner.acceptPeginTxHash
        );
        // We need to wait for the event to be confirmed before processing it
        self.track_pegin_requested(pegin_flow_id, pegin_requested)
    }

    fn handle_pegin_accepted(&mut self, data: &PeginAcceptedEvent) -> Result<()> {
        if data.removed {
            debug!(
                "Removing PeginAccepted: acceptPeginTxHash={}",
                data.inner.acceptPeginTxHash
            );
            return self.untrack_pegin_accepted(data.clone());
        }

        let tx_hash: TxHash = data.inner.acceptPeginTxHash.into();
        let Some(state) = self.tracker.get(&tx_hash) else {
            bail!(
                "Received PeginAccepted for unknown acceptPeginTxHash: {:?}",
                tx_hash
            );
        };

        let committee_id: CommitteeId = state.pegin_requested.data.inner.committeeId.try_into()?;
        if !self.global_context.my_committees().im_member(&committee_id) {
            trace!(
                "Skipping PeginAccepted: not_committee_member, acceptPeginTxHash={}",
                data.inner.acceptPeginTxHash
            );
            return Ok(());
        }
        info!(
            "Processing PeginAccepted: acceptPeginTxHash={}",
            data.inner.acceptPeginTxHash
        );

        let observer_id = format!("pegin_accepted-{}", state.flow_id);
        let confirmations =
            BlockConfirmations::new(observer_id, data.block_number, REQUIRED_CONFIRMATIONS);
        let pegin_accepted = PeginEvent::new(data.clone(), confirmations);

        self.blockchain
            .add_observer(pegin_accepted.confirmations.clone());

        debug!(
            "Tracking PeginAccepted: flow_id={}, acceptPeginTxHash={}",
            state.flow_id, data.inner.acceptPeginTxHash
        );

        self.track_pegin_accepted(pegin_accepted)
    }

    fn handle_all_operator_take_tx_hashes_added_event(
        &mut self,
        data: &AllOperatorTakeTxHashesAddedEvent,
    ) -> Result<()> {
        let tx_hash: TxHash = data.inner.acceptPeginTxHash.into();

        // Check if the key exists first to avoid unnecessary operations
        if !self.tracker.contains_key(&tx_hash) {
            debug!(
                "Received AllOperatorTakeTxHashesAdded: unknown_acceptPeginTxHash={:?}",
                tx_hash
            );
            return Ok(());
        }

        if data.removed {
            self.untrack_all_operator_take_tx_hashes_added(data.inner.clone())?;
            return Ok(()); // Stop here if removed
        }

        debug!(
            "Processing AllOperatorTakeTxHashesAdded: acceptPeginTxHash={}",
            data.inner.acceptPeginTxHash
        );

        let Some(state) = self.tracker.get(&tx_hash) else {
            bail!("State should exist for tx_hash {tx_hash:?} after AllOperatorTakeTxHashesAdded");
        };

        let committee_id: CommitteeId = state.pegin_requested.data.inner.committeeId.try_into()?;
        if !self.global_context.my_committees().im_member(&committee_id) {
            trace!(
                "Skipping AllOperatorTakeTxHashesAdded: not_committee_member, acceptPeginTxHash={}",
                data.inner.acceptPeginTxHash
            );
            return Ok(());
        }
        info!(
            "Processing AllOperatorTakeTxHashesAdded: acceptPeginTxHash={}",
            data.inner.acceptPeginTxHash
        );

        let observer_id = format!("operator_take_tx_hashes_added-{}", state.flow_id);
        debug!(
            "Tracking AllOperatorTakeTxHashesAdded: flow_id={}, acceptPeginTxHash={}",
            state.flow_id, data.inner.acceptPeginTxHash
        );
        let confirmations =
            BlockConfirmations::new(observer_id, data.block_number, REQUIRED_CONFIRMATIONS);
        let operator_take_tx_hashes_added = PeginEvent::new(data.clone(), confirmations);
        self.track_all_operator_take_tx_hashes_added(operator_take_tx_hashes_added.clone())?;

        self.blockchain
            .add_observer(operator_take_tx_hashes_added.confirmations);

        Ok(())
    }

    fn track_all_operator_take_tx_hashes_added(
        &mut self,
        event: PeginEvent<AllOperatorTakeTxHashesAdded>,
    ) -> Result<()> {
        let tx_hash: TxHash = event.data.inner.acceptPeginTxHash.into();

        if let Some(state) = self.tracker.get_mut(&tx_hash) {
            state.all_operators_take_tx_hashes_added = Some(event);
            Ok(())
        } else {
            bail!("AllOperatorTakeTxHashesAdded cannot be found for tx_hash: {tx_hash:?}");
        }
    }

    fn untrack_all_operator_take_tx_hashes_added(
        &mut self,
        event: AllOperatorTakeTxHashesAdded,
    ) -> Result<()> {
        let tx_hash: TxHash = event.acceptPeginTxHash.into();
        match self.tracker.get_mut(&tx_hash) {
            Some(state) => {
                if let Some(operators_take_tx_hashes) = &state.all_operators_take_tx_hashes_added {
                    let confirmations = operators_take_tx_hashes.confirmations.borrow();
                    let observer_id = confirmations.get_id();
                    self.blockchain.remove_observer(observer_id.as_str());
                } else {
                    bail!(
                        "Trying to untrack AllOperatorTakeTxHashesAdded event, but tracker entry for tx_hash: {:?} has no AllOperatorTakeTxHashesAdded event",
                        tx_hash
                    );
                }
                state.all_operators_take_tx_hashes_added = None;
                debug!(
                    "Untracked AllOperatorTakeTxHashesAdded: tx_hash={:?}, flow_id={}",
                    tx_hash, state.flow_id
                );
                Ok(())
            }
            None => {
                bail!(
                    "Expected to untrack AllOperatorTakeTxHashesAdded event but no entry found for tx_hash: {:?}",
                    tx_hash
                );
            }
        }
    }

    fn handle_all_operator_take_tx_hashes_added(&mut self, tx_hash: &TxHash) -> Result<()> {
        let event: AllOperatorTakeTxHashesAdded = self
            .tracker
            .get(tx_hash)
            .and_then(|state| state.all_operators_take_tx_hashes_added.as_ref())
            .ok_or_else(|| {
                anyhow!(
                    "AllOperatorTakeTxHashesAdded event not found for tx_hash: {:?}",
                    tx_hash
                )
            })?
            .data
            .inner
            .clone();

        debug!(
            "Processing AllOperatorTakeTxHashesAdded event: acceptPeginTxHash={}",
            event.acceptPeginTxHash
        );
        // Find the pegin state using the accept_pegin_tx_hash from the event
        let accept_pegin_tx_hash: TxHash = event.acceptPeginTxHash.into();

        if let Some(state) = self.tracker.get_mut(&accept_pegin_tx_hash) {
            let flow_id = state.flow_id;

            // Step 6a Start the signatures sub-flow if not already started
            if state.btc_signatures_flow.is_none() {
                info!("Starting BTC signature flow: flow_id={}", flow_id);
                let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(flow_id);

                let pegin_accepted = state.bitvmx_pegin_accepted.as_ref().ok_or_else(|| {
                    anyhow!("PeginAcceptedMessage not found for flow_id: {}.", flow_id)
                })?;

                let register_input =
                    RegisterSignaturesBitVmxData::try_from(pegin_accepted.clone())?;
                btc_sig_subflow.start_signature_flow(flow_id, &register_input)?;

                state.btc_signatures_flow = Some(btc_sig_subflow);
            } else {
                error!("BTC signature flow already started: flow_id={}", flow_id);
            }
        } else {
            debug!(
                "Received AllOperatorTakeTxHashesAdded: unknown_acceptPeginTxHash={:?}",
                accept_pegin_tx_hash
            );
        }

        Ok(())
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

        // Send GetCommInfo message to get my P2P address
        Self::send_to_bitvmx(
            &self.bitvmx_broker,
            IncomingBitVMXApiMessages::GetCommInfo(),
        )?;

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

                debug!(
                    "Untracked PeginRequested: tx_hash={:?}, flow_id={}",
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

                debug!(
                    "Untracked PeginAccepted: tx_hash={:?}, flow_id={}",
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
        // Extract references to avoid borrowing conflicts
        let rt_sync = &self.rt_sync;
        let contracts = &self.contracts;
        let bitvmx_broker = &self.bitvmx_broker;

        for (tx_hash, state) in self.tracker.iter_mut() {
            let event = &mut state.pegin_requested;

            // Skip if event is not ready for processing
            if !event.is_confirmed() || event.is_handled {
                continue;
            }

            let flow_id = state.flow_id;
            let pegin_event = &event.data.inner;

            // Build the pegin request message
            let pegin_request =
                Self::build_pegin_request_bitvmx_message(rt_sync, contracts, pegin_event)?;

            // Step 4a Send to BitVMX
            Self::send_bitvmx_variable(bitvmx_broker, flow_id, PEGIN_REQUEST, &pegin_request)
                .context(format!(
                    "Error processing confirmed PeginRequested event (tx_hash: {}, flow_id: {})",
                    tx_hash, flow_id
                ))?;

            // Step 4b Send Setup message right after PeginRequestMessage
            Self::send_setup_message(
                rt_sync,
                contracts,
                bitvmx_broker,
                flow_id,
                pegin_event,
                state.my_p2p_address.as_ref(),
            )
            .context(format!(
                "Error sending Setup message (tx_hash: {}, flow_id: {})",
                tx_hash, flow_id
            ))?;

            // Mark event as handled and clean up
            event.mark_handled();

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());

            debug!("Processed confirmed PeginRequested: flow_id={}", flow_id);
        }

        Ok(())
    }

    fn build_pegin_request_bitvmx_message(
        rt_sync: &RuntimeSync,
        contracts: &CG,
        pegin_event: &PeginRequested,
    ) -> Result<PeginRequestMessage> {
        let committee_id: CommitteeId = pegin_event.committeeId.try_into()?;
        // Get committee information
        let committee_response = Self::call_contract(rt_sync, "getCommittee", || async {
            contracts
                .get_committee(GetCommitteeInput {
                    committee_id: committee_id.clone(),
                })
                .await
        })?;

        let operator_indexes = Self::build_operator_indexes(&committee_response)?;

        let slot_index: u64 = pegin_event.streamPosition.slotId;

        let checksum_address = pegin_event
            .requestPeginInfo
            .rskDestinationAddress
            .to_checksum(None);
        let rootstock_address = checksum_address
            .get(2..)
            .ok_or_else(|| anyhow!("RSK address checksum too short"))?;

        let accept_pegin_sighash = pegin_event.acceptPeginSignatureMessage.to_vec();

        let take_aggregated_key = Self::build_take_aggregated_key(&committee_response)?;

        let reimbursement_pubkey = Self::build_reimbursement_pubkey(pegin_event)?;

        let txid = TxIdParser::fb_32_to_txid(pegin_event.requestPeginTxHash);

        let committee_uuid = Self::build_committee_id(committee_id)?;

        Ok(PeginRequestMessage {
            txid,
            amount: pegin_event.prevoutData.value,
            accept_pegin_sighash,
            take_aggregated_key,
            operator_indexes,
            slot_index,
            committee_id: committee_uuid,
            rootstock_address: rootstock_address.to_string(),
            reimbursement_pubkey,
        })
    }

    fn build_operator_indexes(committee_response: &GetCommitteeOutput) -> Result<Vec<usize>> {
        let operator_role: u8 = Prover.into();
        let mut operator_indexes = Vec::new();

        for (i, member) in committee_response.committee.members.iter().enumerate() {
            if member.role != operator_role {
                continue;
            }

            operator_indexes.push(i);
        }

        Ok(operator_indexes)
    }

    fn build_take_aggregated_key(committee_response: &GetCommitteeOutput) -> Result<PublicKey> {
        // aggregatedKey comes with parity (33 bytes), parse directly as PublicKey
        PublicKey::from_slice(&committee_response.committee.aggregatedKey)
            .context("Failed to parse aggregated public key from committee")
    }

    fn build_reimbursement_pubkey(pegin_event: &PeginRequested) -> Result<PublicKey> {
        let reimbursement_xonly_key = XOnlyPublicKey::from_slice(
            pegin_event
                .requestPeginInfo
                .btcReimbursementPubKey
                .as_slice(),
        )
        .context("Failed to parse reimbursement public key from pegin event")?;
        let reimbursement_secp_key = reimbursement_xonly_key.public_key(Even);
        Ok(PublicKey::new(reimbursement_secp_key))
    }

    fn build_committee_id(committee_id: CommitteeId) -> Result<Uuid> {
        Ok(Uuid::from_u128(*committee_id))
    }

    //Step 10 after confirmation of the pegin accepted event, we need to send the
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

            debug!("Processed confirmed PeginAccepted: flow_id={}", flow_id);

            to_remove.push((*tx_hash, flow_id));
        }

        // Pegin completed so we can remove the state in tracker
        for (tx_hash, flow_id) in to_remove {
            info!(
                "Pegin completed: tx_hash={:?}, flow_id={}",
                tx_hash, flow_id
            );
            self.tracker.remove(&tx_hash);
            if self.tracker.is_empty() {
                debug!("Stopping processor no more events");
                self.scheduler.clear();
                self.blockchain.clear();
            }
        }

        Ok(())
    }

    fn process_unhandled_confirmed_all_operator_take_tx_hashes_added_events(
        &mut self,
    ) -> Result<()> {
        let mut to_handle = Vec::new();

        for (tx_hash, state) in self.tracker.iter_mut() {
            let event = match state.all_operators_take_tx_hashes_added.as_mut() {
                None => continue,
                Some(event) if !event.is_confirmed() || event.is_handled => continue,
                Some(event) => event,
            };

            event.mark_handled();

            let confirmations = event.confirmations.borrow();
            let observer_id = confirmations.get_id();
            self.blockchain.remove_observer(observer_id.as_str());
            debug!(
                "Processed confirmed AllOperatorTakeTxHashesAdded: flow_id={}",
                state.flow_id
            );
            to_handle.push(*tx_hash);
        }
        // Process the collected tx_hashes after the loop to avoid borrowing conflicts
        for tx_hash in to_handle {
            self.handle_all_operator_take_tx_hashes_added(&tx_hash)?;
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

    fn is_pegin_request_tracked(&self, tx_id: &Txid) -> bool {
        self.pegin_request_tracker.contains(tx_id)
    }

    fn handle_pegin_transaction_found(&mut self, tx_id: Txid) -> Result<()> {
        self.pegin_request_tracker.insert(tx_id);
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

    fn send_dispatch_transaction_name(bitvmx_broker: &BC, flow_id: Uuid) -> Result<()> {
        let message = IncomingBitVMXApiMessages::DispatchTransactionName(
            flow_id,
            ACCEPT_PEGIN_TX.to_string(),
        );
        Self::send_to_bitvmx(bitvmx_broker, message)
    }

    fn send_get_transaction(bitvmx_broker: &BC, flow_id: Uuid, tx_id: Txid) -> Result<()> {
        let message = IncomingBitVMXApiMessages::GetTransaction(flow_id, tx_id);
        Self::send_to_bitvmx(bitvmx_broker, message)
    }

    fn send_get_spv_proof_to_bitvmx(bitvmx_broker: &BC, tx_id: Txid) -> Result<()> {
        trace!("Requesting SPV proof: tx_id={}", tx_id);
        let msg = IncomingBitVMXApiMessages::GetSPVProof(tx_id);
        Self::send_to_bitvmx(bitvmx_broker, msg)
    }

    fn send_to_bitvmx(bitvmx_broker: &BC, message: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("Sending BitVMX message: type={:?}", message);

        bitvmx_broker.send(BROKER_SERVER_ID, message)?;

        Ok(())
    }

    fn get_committee_addresses(
        rt_sync: &RuntimeSync,
        contracts: &CG,
        committee_id: &CommitteeId,
    ) -> Result<Vec<String>> {
        let input = GetCommunicationDataInput {
            committee_id: committee_id.clone(),
            member_address: contracts.my_address().into(),
        };
        let communication_data_response =
            Self::call_contract(rt_sync, "getMemberCommunicationData", || async {
                contracts.get_committee_communication_data(input).await
            })?;

        let committee_addresses = communication_data_response
            .communication_data
            .into_iter()
            .map(|comm_data| {
                P2PAddressParser::addr_from_contracts(&comm_data)
                    .context("Failed to convert communication data to P2P address")
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(committee_addresses)
    }

    fn get_committee_peer_ids(
        rt_sync: &RuntimeSync,
        contracts: &CG,
        committee_id: &CommitteeId,
    ) -> Result<Vec<PeerId>> {
        let committee_input = GetCommitteeInput {
            committee_id: committee_id.clone(),
        };
        let committee_response = Self::call_contract(rt_sync, "getCommittee", || async {
            contracts.get_committee(committee_input).await
        })?;

        let mut peer_ids = Vec::new();

        for member in committee_response.committee.members {
            // Get the member's public keys
            let keys_input = GetMemberPublicKeysInput {
                member_address: member.memberAddress,
            };

            let keys_response = Self::call_contract(rt_sync, "getMemberPublicKeys", || async {
                contracts.get_member_public_keys(keys_input).await
            })?;

            // Get the communication key (at index 2)
            let key_str = keys_response
                .public_keys
                .get(COMM_KEY_INDEX)
                .context(format!(
                    "Communication key not found for member {}",
                    member.memberAddress
                ))?;

            debug!(
                "Member PeerId: address={}, peer_id={:?}",
                member.memberAddress, key_str
            );
            peer_ids.push(PeerId(key_str.to_string()));
        }

        Ok(peer_ids)
    }

    fn send_setup_message(
        rt_sync: &RuntimeSync,
        contracts: &CG,
        bitvmx_broker: &BC,
        flow_id: Uuid,
        pegin_event: &PeginRequested,
        my_p2p_address: Option<&P2PAddress>,
    ) -> Result<()> {
        // Ensure my_p2p_address is available
        let my_addr = my_p2p_address.ok_or_else(|| {
            anyhow!(
                "my_p2p_address not yet available for flow_id {}. Cannot send Setup message.",
                flow_id
            )
        })?;

        let committee_id: CommitteeId = pegin_event.committeeId.try_into()?;

        // Get committee addresses
        let committee_addresses = Self::get_committee_addresses(rt_sync, contracts, &committee_id)?;

        // Get committee peer IDs
        let committee_peer_ids = Self::get_committee_peer_ids(rt_sync, contracts, &committee_id)?;

        // Use build_communication_data to construct the P2P addresses
        let p2p_addresses = build_communication_data(
            my_addr.address.clone(),
            committee_addresses,
            committee_peer_ids,
        )?;

        debug!("P2P addresses for Setup: addresses={:?}", p2p_addresses);

        let setup_message = IncomingBitVMXApiMessages::Setup(
            flow_id,                               // ProgramId - UUID of pegin flow
            PROGRAM_TYPE_ACCEPT_PEGIN.to_string(), // Program type constant
            p2p_addresses,                         // Vector of P2P addresses
            0,                                     // Leader number
        );

        Self::send_to_bitvmx(bitvmx_broker, setup_message)
    }

    fn send_accept_pegin_contracts(
        &mut self,
        spv_proof: BtcTxSPVProof,
        flow_id: Uuid,
        attempt: i16,
    ) -> Result<()> {
        debug!("Registering SPV proof: spv_proof={:?}", spv_proof);

        let input = spv_proof.clone().into();

        // Step 10a Call the contract to register the SPV proof
        match invoke_contract_safe(
            &self.rt_sync,
            ACCEPT_PEGIN,
            &spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.accept_pegin(input).await },
        ) {
            Ok(_) => {
                // Remove from unconfirmed (idempotent - no-op if not present)
                self.unconfirmed_pegin_accepts.remove(&flow_id);
                debug!(
                    "Removed from unconfirmed_pegin_accepts: flow_id={}",
                    flow_id
                );
                Ok(())
            }
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(_)) => {
                // Schedule retry and return error
                self.schedule_pegin_accept_retry(flow_id, spv_proof, attempt + 1);
                bail!("Insufficient confirmations, retry scheduled")
            }
            Err(e) => Err(e.into()),
        }
    }

    fn handle_bitvmx_pegin_accepted(
        &mut self,
        flow_id: Uuid,
        pegin_accepted: PeginAcceptedMessage,
    ) -> Result<()> {
        debug!(
            "Processing PeginAcceptedMessage: flow_id={}, acceptPeginTxHash={}",
            flow_id, pegin_accepted.accept_pegin_txid
        );

        // Find the pegin state by flow_id and save the PeginAcceptedMessage data
        let Some(state) = self
            .tracker
            .values_mut()
            .find(|state| state.flow_id == flow_id)
        else {
            bail!(
                "No pegin state found for flow_id: {}. Cannot save PeginAcceptedMessage data.",
                flow_id
            );
        };

        let accept_pegin_tx_hash = pegin_accepted.accept_pegin_txid;

        let take_tx_hash = pegin_accepted.operator_take_sighash.clone();

        state.bitvmx_pegin_accepted = Some(pegin_accepted);
        debug!("Saved PeginAcceptedMessage: flow_id={}", flow_id);

        // Deposit the operator take tx hash as soon as we receive PeginAcceptedMessage
        debug!(
            "Adding operator take tx: flow_id={}, acceptPeginTxHash={}",
            flow_id, accept_pegin_tx_hash
        );

        let input = AddOperatorTakeTxHashInput {
            accept_pegin_tx_hash,
            take_tx_hash,
        };
        // Step 5a Call the addOperatorTakeTxHash contract method
        invoke_contract(&self.rt_sync, "addOperatorTakeTxHash", || async {
            self.contracts.add_operator_take_tx_hash(input).await
        })?;

        Ok(())
    }

    fn send_request_pegin_contracts(
        &mut self,
        spv_proof: BtcTxSPVProof,
        attempt: i16,
        tx_id_to_track: Option<&Txid>,
        btc_block_to_unconfirm: Option<String>,
    ) -> Result<()> {
        let input: RequestPeginInput = spv_proof.clone().into();

        // Step 3a Call the requestPegin contract method
        match invoke_contract_safe(
            &self.rt_sync,
            "requestPegin",
            &spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.request_pegin(input).await },
        ) {
            Ok(_) => {
                self.cleanup_request_pegin_tracking(tx_id_to_track, btc_block_to_unconfirm);
                Ok(())
            }
            // todo(fede) i decided to comment this and will remove it when we confirm that is ok to
            // remove it
            // Err(DomainErrors::MissingConfirmationsOnNativeBridge(_)) => {
            //     // Schedule retry
            //     self.schedule_pegin_requested_to_contracts(spv_proof, attempt + 1);
            //     // Remove from tracker if provided (retry is now in scheduler)
            //     self.cleanup_request_pegin_tracking(tx_id_to_track, None);
            //     bail!("Insufficient confirmations, retry scheduled")
            // }
            Err(DomainErrors::PeginAlreadyRequested(msg)) => {
                // This is expected if the same pegin is requested multiple times
                // We should treat it as a success case
                let tx_id = spv_proof.tx.compute_txid();
                info!(
                    "Pegin already requested for tx_id={}, treating as expected: {}",
                    tx_id, msg
                );
                self.cleanup_request_pegin_tracking(tx_id_to_track, btc_block_to_unconfirm);
                Ok(())
            }
            Err(domain_err) => bail!("Error executing 'requestPegin': {:?}", domain_err),
        }
    }

    fn cleanup_request_pegin_tracking(
        &mut self,
        tx_id_to_track: Option<&Txid>,
        btc_block_to_unconfirm: Option<String>,
    ) {
        if let Some(tx_id) = tx_id_to_track {
            self.pegin_request_tracker.remove(tx_id);
            trace!("Removed request_pegin_txid from tracking: tx_id={tx_id}");
        }
        if let Some(btc_block) = btc_block_to_unconfirm {
            self.unconfirmed_pegin_requests.remove(&btc_block);
            trace!("Removed from unconfirmed_pegin_requests: btc_block={btc_block}",);
        }
    }

    fn call_contract<Fut, F, T>(rt_sync: &RuntimeSync, method_name: &str, call: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, DomainErrors>>,
        T: Debug,
    {
        debug!("Calling contract method: method={}", method_name);

        match rt_sync.run(call()) {
            Ok(result) => {
                debug!(
                    "Contract method called: method={}, result={:?}",
                    method_name, result
                );
                Ok(result)
            }
            Err(domain_err) => {
                bail!("Error calling '{}': {:?}", method_name, domain_err)
            }
        }
    }

    fn handle_transaction_status_received(
        &mut self,
        flow_id: &Uuid,
        tx_status: TransactionStatus,
    ) -> Result<()> {
        debug!(
            "Handling transaction status: status={:?}, flow_id={}",
            tx_status, flow_id
        );
        // find the pegin event state for the given flow_id
        let Some(state) = self
            .tracker
            .values_mut()
            .find(|state| state.flow_id == *flow_id)
        else {
            debug!("No pegin state found: flow_id={}", flow_id);
            return Ok(());
        };

        if let Some(pegin_accepted) = state.bitvmx_pegin_accepted.as_ref() {
            if pegin_accepted.accept_pegin_txid != tx_status.tx_id {
                bail!(
                    "Pegin state for flow_id: {} does not match tx_id: {}",
                    flow_id,
                    tx_status.tx_id
                );
            }
        } else {
            bail!("No pegin accepted message found for flow_id: {}", flow_id);
        }

        let action_key = ScheduledAction::PeginAccepted(*flow_id);

        if tx_status.confirmations >= MIN_TX_CONFIRMATIONS {
            // Step 9b confirmation enough, send the SPV proof to BitVMX
            debug!(
                "Transaction confirmed: confirmations={}, flow_id={}",
                tx_status.confirmations, flow_id
            );

            if self.scheduler.is_scheduled(&action_key) {
                debug!("Unscheduling get transaction: flow_id={}", flow_id);
                self.scheduler.cancel(&action_key);
            }
            Self::send_get_spv_proof_to_bitvmx(&self.bitvmx_broker, tx_status.tx_id)?;
        } else {
            //step 9a confirmation not enough, reschedule the check
            debug!(
                "Transaction {} has not enough confirmations [{}/{MIN_TX_CONFIRMATIONS}] for flow_id={flow_id}",
                tx_status.tx_id, tx_status.confirmations
            );
            debug!("Scheduling get transaction: flow_id={}", flow_id);
            self.scheduler
                .schedule(action_key, BLOCKS_DELAY_FOR_TX_CHECK);
        }

        Ok(())
    }

    fn process_unhandled_confirmed_sig_flow_events(
        &mut self,
        block: &RskBlockAndUncles,
    ) -> Result<()> {
        for (_, state) in self.tracker.iter_mut() {
            if let Some(btc_flow) = state.btc_signatures_flow.as_mut() {
                btc_flow.delegate_block(block)?;
                if btc_flow.is_done() {
                    //#Step 8a: Send DispatchTransaction to BitVMX
                    Self::send_dispatch_transaction_name(&self.bitvmx_broker, state.flow_id)?;
                    debug!(
                        "Dispatch tx sent for accept pegin: flow_id={}",
                        state.flow_id
                    );
                    //Signature flow is done, we can clear it from state
                    state.btc_signatures_flow = None;
                }
            }
        }
        Ok(())
    }

    fn schedule_pegin_requested_to_contracts(&mut self, spv_proof: BtcTxSPVProof, attempt: i16) {
        self.unconfirmed_pegin_requests
            .insert(spv_proof.block_hash.clone(), spv_proof.clone());

        // Native bridge has not yet enough confirmations, we need to retry later
        self.scheduler.schedule(
            ScheduledAction::PeginRequested(spv_proof.block_hash, attempt),
            BLOCKS_DELAY_FOR_TX_CHECK,
        );
    }

    fn schedule_pegin_accept_retry(
        &mut self,
        flow_id: Uuid,
        spv_proof: BtcTxSPVProof,
        attempt: i16,
    ) {
        self.unconfirmed_pegin_accepts
            .insert(flow_id, spv_proof.clone());

        info!(
            "Scheduling accept_pegin retry for flow_id={}, attempt={}",
            flow_id, attempt
        );

        self.scheduler.schedule(
            ScheduledAction::PeginAcceptRetry(flow_id, attempt),
            BLOCKS_DELAY_FOR_TX_CHECK,
        );
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
            // Step 2: Handle PeginRequested event from BitVMX
            OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, _tx_status) => {
                debug!("Received BitVMX PeginTransactionFound: tx_id={}", tx_id);

                self.handle_pegin_transaction_found(*tx_id)?;
                //TODO in the future we need to validate the tx_statos.confirmations number.
            }
            // Step 3: Handle SPVProof event from BitVMX to call requestPegin
            // Step 10: Handle SPVProof event from BitVMX to call acceptPegin
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => match spv_proof_opt {
                Some(spv_proof) => {
                    debug!("Received BitVMX SPVProof: tx_id={}", tx_id);
                    trace!(
                        "Received spv_proof_data for tx_id={}: {:?}",
                        tx_id, spv_proof
                    );
                    // Step 3.1: Handle request pegin SPV proof
                    if self.is_pegin_request_tracked(tx_id) {
                        info!("Handling request pegin SPV proof: tx_id={}", tx_id);
                        self.send_request_pegin_contracts(spv_proof.clone(), 1, Some(tx_id), None)?;
                    }
                    // Find state by matching accept_pegin_txid from bitvmx_pegin_accepted
                    let matching_state = self.tracker.iter_mut().find(|(_, state)| {
                        state
                            .bitvmx_pegin_accepted
                            .as_ref()
                            .map(|accepted| accepted.accept_pegin_txid == *tx_id)
                            .unwrap_or(false)
                    });

                    if let Some((_, state)) = matching_state {
                        let flow_id = state.flow_id;
                        // Step 10 Handle accept pegin SPV proof
                        info!(
                            "Handling accept pegin SPV proof: flow_id={}, tx_id={}",
                            flow_id, tx_id
                        );

                        self.send_accept_pegin_contracts(spv_proof.clone(), flow_id, 1)?
                    }
                    debug!(
                        "SPV proof for tx_id: {} is not related to a pegin flow",
                        tx_id
                    );
                }
                None => bail!(
                    "Received BitVMX SPVProof event for tx_id: {}, but no SPV proof was included.",
                    tx_id
                ),
            },
            // Step 5: Handle PeginAccepted event from BitVMX
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), PEGIN_ACCEPTED_INPUT_MSG) =>
            {
                debug!(
                    "Received BitVMX Variable pegin_accepted: flow_id={}, method={}, payload={:?}",
                    flow_id, method, data
                );

                let pegin_accepted: PeginAcceptedMessage = serde_json::from_str(data)
                    .with_context(|| {
                        format!(
                            "Failed to deserialize PeginAcceptedMessage from BitVMX message {data}"
                        )
                    })?;

                self.handle_bitvmx_pegin_accepted(*flow_id, pegin_accepted)?;
            }
            // Step 9: Handle Transaction event from BitVMX and check confirmation status
            OutgoingBitVMXApiMessages::Transaction(flow_id, tx_status, _tx_opt) => {
                debug!(
                    "Received BitVMX Transaction event: flow_id={}, tx_status={:?}",
                    flow_id, tx_status
                );
                self.handle_transaction_status_received(flow_id, tx_status.clone())?;
            }
            OutgoingBitVMXApiMessages::CommInfo(p2p_address) => {
                // Find the first pegin state that doesn't have my_p2p_address set yet
                if let Some((_, state)) = self
                    .tracker
                    .iter_mut()
                    .find(|(_, state)| state.my_p2p_address.is_none())
                {
                    state.my_p2p_address = Some(p2p_address.clone());
                    debug!(
                        "Set my_p2p_address: flow_id={}, address={:?}",
                        state.flow_id, p2p_address
                    );
                } else {
                    trace!("Ignoring BitVMX CommInfo")
                }
            }
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                // Check if there is any UUID in the state matching the ProgramId
                if let Some((_, state)) = self
                    .tracker
                    .iter()
                    .find(|(_, state)| state.flow_id == *program_id)
                {
                    info!(
                        "BitVMX setup for pegin was completed: flow_id={}",
                        state.flow_id
                    );
                } else {
                    trace!(
                        "Ignoring BitVMX SetupCompleted for unknown program_id: {}",
                        program_id
                    );
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        info!(
            "Committee membership: committees={:?}",
            self.global_context.my_committees()
        );

        match event {
            // Step 4: Handle PeginRequested event from RSK
            RskPegManagerEvents::PeginRequested(data) => self.handle_pegin_requested(data),
            //Step 11: Handle PeginAccepted event from RSK
            RskPegManagerEvents::PeginAccepted(data) => self.handle_pegin_accepted(data),
            //Step 6: Handle AllOperatorTakeTxHashesAdded event from RSK
            RskPegManagerEvents::AllOperatorTakeTxHashesAdded(data) => {
                self.handle_all_operator_take_tx_hashes_added_event(data)
            }
            //Step 7: Handle AllNoncesReady and AllSignaturesReady event from RSK
            RskPegManagerEvents::AllNoncesReady(_data)
            //Step 8: Handle AllSignaturesReady event from RSK
            | RskPegManagerEvents::AllSignaturesReady(_data) => {
                for state in self.tracker.values_mut() {
                    let committee_id: &CommitteeId = &state.pegin_requested.data.inner.committeeId.try_into()?;
                    if !self.global_context.my_committees().im_member(&committee_id) {
                        debug!(
                            "Skipping signature flow delegation: not a member of committee_id={:?}",
                            committee_id
                        );
                        continue;
                    }

                    if let Some(btc_sig_flow) = &mut state.btc_signatures_flow {
                        // Delegate the event to the BTC signature flow
                        btc_sig_flow.delegate_rsk_event(state.flow_id, event)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.blockchain.update(block.clone());

        self.tick_scheduler()?;

        if self.tracker.is_empty() {
            return Ok(());
        }

        self.process_unhandled_confirmed_pegin_requested_events()?;
        self.process_unhandled_confirmed_all_operator_take_tx_hashes_added_events()?;
        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.process_unhandled_confirmed_pegin_accepted_events()?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PeginProcessor");

        self.scheduler.clear();
        self.blockchain.clear();
        self.tracker.clear();
    }
}

fn invoke_contract<Fut, F, T>(
    rt_sync: &RuntimeSync,
    method_name: &str,
    invoke: F,
) -> Result<T, DomainErrors>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, DomainErrors>>,
{
    debug!("Submitting contract transaction: method={}", method_name);

    match rt_sync.run(invoke()) {
        Ok(value) => {
            debug!("Contract method executed: method={}", method_name);
            Ok(value)
        }
        Err(domain_err) => Err(domain_err),
    }
}

// verifies that the native bridge has enough confirmations
// for the given spv proof and then invokes a contract
fn invoke_contract_safe<Fut, F, T, CG>(
    rt_sync: &RuntimeSync,
    method_name: &str,
    spv_proof: &BtcTxSPVProof,
    native_bridge_verifier: &NativeBridgeVerifier<CG>,
    invoke: F,
) -> Result<T, DomainErrors>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, DomainErrors>>,
    CG: RskContractsGatewayApi,
{
    debug!(
        "Verifying Native Bridge confirmations before invoking: method={}",
        method_name
    );

    match native_bridge_verifier.verify_confirmations(spv_proof, MIN_TX_CONFIRMATIONS)? {
        VerificationStatus::Verified => invoke_contract(rt_sync, method_name, invoke),
        VerificationStatus::InsufficientConfirmations { required, actual } => {
            debug!(
                "Insufficient Native Bridge confirmations for {}: {}/{} - needs retry",
                method_name, actual, required
            );
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(format!(
                "{}/{} confirmations",
                actual, required
            )))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerificationStatus {
    Verified,
    InsufficientConfirmations { required: u32, actual: u32 },
}

pub enum NativeBridgeVerifier<CG: RskContractsGatewayApi> {
    Real {
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
    },
    Dummy, // used in local/test environments
}

impl<CG: RskContractsGatewayApi> NativeBridgeVerifier<CG> {
    fn verify_confirmations(
        &self,
        spv_proof: &BtcTxSPVProof,
        required_confirmations: u32,
    ) -> Result<VerificationStatus, DomainErrors> {
        match self {
            NativeBridgeVerifier::Real { contracts, rt_sync } => {
                verify_btc_confirmations(spv_proof, required_confirmations, contracts, rt_sync)
            }
            NativeBridgeVerifier::Dummy => {
                trace!(
                    "Using dummy verifier: skipping Native Bridge verification (local environment)"
                );
                Ok(VerificationStatus::Verified)
            }
        }
    }
}

fn verify_btc_confirmations<CG>(
    spv_proof: &BtcTxSPVProof,
    required_confirmations: u32,
    contracts: &Rc<CG>,
    rt_sync: &RuntimeSync,
) -> std::result::Result<VerificationStatus, DomainErrors>
where
    CG: RskContractsGatewayApi,
{
    use transaction_dispatcher::types::GetBtcTransactionConfirmationsInput;

    // todo(fede) review this
    let block_hash: common::types::BlockHash =
        match spv_proof.block_hash.parse::<primitive_types::H256>() {
            Ok(h) => h.into(),
            Err(e) => {
                warn!("Failed to parse block hash: {}", e);
                return Err(DomainErrors::InvalidBtcTxSpvProof(format!(
                    "Invalid block hash: {}",
                    e
                )));
            }
        };

    let tx_id = spv_proof.tx.compute_txid();
    let tx_hash_fb = TxIdParser::txid_to_fb_32(tx_id);
    let tx_hash: common::types::TxHash =
        common::types::Hash256::from(primitive_types::H256::from_slice(tx_hash_fb.as_slice()));

    let merkle_branch_hashes: Vec<String> = spv_proof
        .merkle_branch_hashes
        .iter()
        .map(|hash| hex::encode(hash))
        .collect();

    let input = GetBtcTransactionConfirmationsInput {
        tx_hash,
        block_hash,
        merkle_branch_path: spv_proof.merkle_branch_path.clone(),
        merkle_branch_hashes,
    };

    // query native bridge
    match invoke_contract(rt_sync, "getBtcConfirmations", || async {
        contracts.get_btc_confirmations(input).await
    }) {
        Ok(output) => {
            let confirmations = output.confirmations;
            if confirmations >= required_confirmations {
                debug!(
                    "Native Bridge verification passed: {}/{} confirmations",
                    confirmations, required_confirmations
                );
                Ok(VerificationStatus::Verified)
            } else {
                info!(
                    "Native Bridge has insufficient confirmations: {}/{}, will retry later",
                    confirmations, required_confirmations
                );
                // Insufficient confirmations is NOT an error, it's an expected state
                Ok(VerificationStatus::InsufficientConfirmations {
                    required: required_confirmations,
                    actual: confirmations,
                })
            }
        }
        Err(e) => {
            warn!("Native Bridge query failed: {}", e);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::btc_signature::btc_signature_subflow::MockBtcSigSubFlowFactory;
    use crate::flows::common::MyCommittees;
    use crate::{
        coordinator::tests::MockRskContractsGatewayApi,
        event_processor::EventProcessor,
        types::{PeginAcceptedEvent, PeginRequestedEvent},
    };
    use alloy_primitives::{Address, Bytes, FixedBytes, U256, address};
    use anyhow::anyhow;
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
        transaction::Version,
    };
    use common::{
        msg_broker::{
            bitvmx_types::{TransactionBlockchainStatus, TransactionStatus},
            broker::{BROKER_SERVER_ID, BrokerError, MockBrokerClientApi},
        },
        test_utils::rsk_block_generator::create_block_and_uncles,
        types,
        types::BlockHash,
    };
    use hex::FromHex;
    use mockall::predicate::{eq, function};
    use primitive_types::H256;
    use serde_json::json;
    use transaction_dispatcher::types::GetCommunicationDataOutput;
    use transaction_dispatcher::{
        rsk_gateway::DomainErrors,
        types::{GetCommitteeOutput, GetMemberPublicKeysOutput, RequestPeginOutput, TxSentOutput},
    };
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        Committee, CommitteeMember, Role,
    };
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        let event = OutgoingBitVMXApiMessages::PeginTransactionFound(txid, status);
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn process_new_bitvmx_pegin_transaction_found_should_add_tx_id_to_tracker() {
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        // Verify tx_id is not tracked initially
        assert!(!processor.is_pegin_request_tracked(&txid));

        let event = OutgoingBitVMXApiMessages::PeginTransactionFound(txid, status);
        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());

        // Verify tx_id is now tracked
        assert!(processor.is_pegin_request_tracked(&txid));
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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
        };
        contracts
            .expect_request_pegin()
            .times(1)
            .returning(move |_| Ok(expected_receipt.clone()));

        // Mock Native Bridge confirmations
        contracts
            .expect_get_btc_confirmations()
            .times(1)
            .returning(|_| {
                Ok(
                    transaction_dispatcher::types::GetBtcTransactionConfirmationsOutput {
                        confirmations: 2,
                    },
                )
            });

        // Prepare broker - expects subscription and GetSPVProof request
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let spv_proof = dummy_spv_proof();
        let tx_id = spv_proof.tx.compute_txid();

        // Expect GetSPVProof to be sent when PeginTransactionFound is received
        broker
            .expect_send()
            .withf(move |to, msg| {
                *to == BROKER_SERVER_ID
                    && matches!(msg, IncomingBitVMXApiMessages::GetSPVProof(id) if id == &tx_id)
            })
            .times(1)
            .returning(|_, _| Ok(true));

        let rt_sync = RuntimeSync::new().unwrap();
        let contracts_rc = Rc::new(contracts);
        let mut processor: PeginProcessor<
            MockRskContractsGatewayApi,
            MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
            crate::flows::btc_signature::btc_signature_subflow::MockBtcSignatureSubFlowApi,
            crate::flows::btc_signature::btc_signature_subflow::MockBtcSignatureSubFlowFactoryApi<
                crate::flows::btc_signature::btc_signature_subflow::MockBtcSignatureSubFlowApi,
            >,
        > = PeginProcessor::new(
            rt_sync.clone(),
            contracts_rc.clone(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Real {
                contracts: contracts_rc,
                rt_sync,
            },
        );

        // First send PeginTransactionFound to add tx_id to tracker
        let status = dummy_transaction_status();
        let pegin_found_event: OutgoingBitVMXApiMessages =
            OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, status);
        let result = processor.process_new_bitvmx_event(&pegin_found_event);
        assert!(result.is_ok());
        assert!(processor.is_pegin_request_tracked(&tx_id));

        // Then send SPV proof event
        let spv_proof_event = OutgoingBitVMXApiMessages::SPVProof(tx_id, Some(spv_proof));

        // Run and assert
        let result = processor.process_new_bitvmx_event(&spv_proof_event);
        assert!(result.is_ok());

        // Verify tx_id was removed from tracker after successful processing
        assert!(!processor.is_pegin_request_tracked(&tx_id));
    }

    #[test]
    fn process_new_bitvmx_spv_proof_event_for_request_pegin_should_fail_on_dispatch_error() {
        // Prepare a mocked contracts gateway that simulates a failure
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts
            .expect_request_pegin()
            .times(1)
            .returning(|_| Err(DomainErrors::UnknownContractError("simulated error".into())));

        // Mock Native Bridge confirmations
        contracts
            .expect_get_btc_confirmations()
            .times(1)
            .returning(|_| {
                Ok(
                    transaction_dispatcher::types::GetBtcTransactionConfirmationsOutput {
                        confirmations: 2,
                    },
                )
            });

        // Prepare broker - expects subscription and GetSPVProof request
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let spv_proof = dummy_spv_proof();
        let tx_id = spv_proof.tx.compute_txid();

        // Expect GetSPVProof to be sent when PeginTransactionFound is received
        broker
            .expect_send()
            .withf(move |to, msg| {
                *to == BROKER_SERVER_ID
                    && matches!(msg, IncomingBitVMXApiMessages::GetSPVProof(id) if id == &tx_id)
            })
            .times(1)
            .returning(|_, _| Ok(true));

        let rt_sync = RuntimeSync::new().unwrap();
        let contracts_rc = Rc::new(contracts);
        let mut processor = PeginProcessor::new(
            rt_sync.clone(),
            contracts_rc.clone(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Real {
                contracts: contracts_rc,
                rt_sync,
            },
        );

        // First send PeginTransactionFound to add tx_id to tracker
        let status = dummy_transaction_status();
        let pegin_found_event = OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, status);
        let result = processor.process_new_bitvmx_event(&pegin_found_event);
        assert!(result.is_ok());
        assert!(processor.is_pegin_request_tracked(&tx_id));

        // Then send SPV proof event
        let spv_proof_event = OutgoingBitVMXApiMessages::SPVProof(tx_id, Some(spv_proof));

        // We expect an error due to contract dispatch failure
        let result = processor.process_new_bitvmx_event(&spv_proof_event);
        assert!(result.is_err());
    }

    #[test]
    fn process_new_bitvmx_pegin_accepted_message_saves_data_and_calls_contract() {
        // Set up the mocked contracts gateway
        let mut contracts = MockRskContractsGatewayApi::new();
        let expected_txid = TxIdParser::fb_32_to_txid([0x11; 32].into());
        contracts
            .expect_add_operator_take_tx_hash()
            .times(1)
            .withf(move |input| {
                input.accept_pegin_tx_hash == expected_txid
                    && input.take_tx_hash == vec![0x12, 0x34, 0x56, 0x78]
            })
            .returning(|_| {
                Ok(TxSentOutput {
                    transaction_hash: "0xabcdef".to_string(),
                })
            });

        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);
        expect_get_comm_info(&mut broker);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(
            rt_sync,
            contracts.into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        // First add a pegin state to track
        let flow_id = Uuid::new_v4();
        let pegin_requested = dummy_pegin_requested_event();
        let confirmations = BlockConfirmations::new(flow_id.to_string(), 1.into(), 0);
        let pegin_event = PeginEvent::new(
            PeginRequestedEvent {
                inner: pegin_requested,
                block_number: 1.into(),
                block_hash: BlockHash::from(H256::from([0xaa; 32])),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(1)),
            },
            confirmations,
        );
        processor
            .track_pegin_requested(flow_id, pegin_event)
            .unwrap();

        // Create a PeginAcceptedMessage payload
        let dummy_txid = TxIdParser::fb_32_to_txid([0x11; 32].into());
        let pegin_accepted_payload = json!({
            "committee_id": flow_id.to_string(),
            "accept_pegin_txid": dummy_txid.to_string(),
            "accept_pegin_sighash": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32],
            "accept_pegin_nonce": "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93",
            "accept_pegin_signature": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            "operator_take_sighash": [18, 52, 86, 120],
            "operator_won_sighash": [171, 205, 239, 18]
        });

        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            PEGIN_ACCEPTED_INPUT_MSG.to_string(),
            VariableTypes::String(pegin_accepted_payload.to_string()),
        );

        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_ok());

        // Verify the data was saved
        let state = processor
            .tracker
            .values()
            .find(|s| s.flow_id == flow_id)
            .expect("Should find state with matching flow_id");
        assert!(state.bitvmx_pegin_accepted.is_some());
        let saved_data = state.bitvmx_pegin_accepted.as_ref().unwrap();
        assert_eq!(saved_data.committee_id, flow_id);
        assert_eq!(
            saved_data.operator_take_sighash,
            vec![0x12, 0x34, 0x56, 0x78]
        );
        assert_eq!(
            saved_data.operator_won_sighash,
            vec![0xab, 0xcd, 0xef, 0x12]
        );
    }

    #[test]
    fn process_new_bitvmx_pegin_accepted_message_fails_when_flow_id_not_found() {
        // Set up the mocked contracts gateway (should not be called)
        let contracts = MockRskContractsGatewayApi::new();

        // Prepare broker and assert it doesn't send anything except pegin subscription
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let rt_sync = RuntimeSync::new().unwrap();
        let mut processor = PeginProcessor::new(
            rt_sync,
            contracts.into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        // Create a PegInAcceptedMessage payload with a random flow_id (not tracked)
        let non_existent_flow_id = Uuid::new_v4();
        let dummy_txid = TxIdParser::fb_32_to_txid([0x11; 32].into());
        let pegin_accepted_payload = json!({
            "committee_id": non_existent_flow_id.to_string(),
            "accept_pegin_txid": dummy_txid.to_string(),
            "accept_pegin_sighash": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32],
            "accept_pegin_nonce": "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93",
            "accept_pegin_signature": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            "operator_take_sighash": [18, 52, 86, 120],
            "operator_won_sighash": [171, 205, 239, 18]
        });

        let event = OutgoingBitVMXApiMessages::Variable(
            non_existent_flow_id,
            PEGIN_ACCEPTED_INPUT_MSG.to_string(),
            VariableTypes::String(pegin_accepted_payload.to_string()),
        );

        let result = processor.process_new_bitvmx_event(&event);
        assert!(result.is_err());

        // Verify the error message
        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(error_msg.contains("No pegin state found for flow_id"));
        assert!(error_msg.contains("Cannot save PeginAcceptedMessage data"));
    }

    #[test]
    fn process_new_event_pegin_requested_event_and_observer() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);
        expect_get_comm_info(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        let pegin_requested = dummy_pegin_requested_event();

        add_to_my_committees(processor.global_context.my_committees(), &pegin_requested);

        let tx_hash: TxHash = pegin_requested.acceptPeginTxHash.into();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested.clone(),
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
        assert!(processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_removed_pegin_requested_event() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);
        expect_get_comm_info(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        let pegin_requested = dummy_pegin_requested_event();

        add_to_my_committees(processor.global_context.my_committees(), &pegin_requested);

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
        assert!(processor.blockchain.has_observer(&observer_id));

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
    fn process_new_event_pegin_requested_event_not_my_committee() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        let pegin_requested = dummy_pegin_requested_event();

        // note: we intentionally do NOT add the committee to my_committees
        // add_to_my_committees(processor.global_context.my_committees(), &pegin_requested);

        let tx_hash: TxHash = pegin_requested.acceptPeginTxHash.into();
        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested.clone(),
            block_number: 123.into(),
            block_hash: BlockHash::from(H256::from([0xaa; 32])),
            removed: false,
            tx_hash: tx_hash.clone(),
        });

        let result = processor.process_new_rsk_event(&event);
        assert!(result.is_ok());

        // event should be ignored since committee is not in my_committees
        assert_eq!(processor.tracker.len(), 0);
        assert!(
            !processor
                .blockchain
                .has_observer(&"pegin_requested-".to_string())
        );
    }

    #[test]
    fn process_new_event_pegin_accepted_event_and_observer() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);
        expect_get_comm_info(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        let pegin_requested = dummy_pegin_requested_event();

        add_to_my_committees(processor.global_context.my_committees(), &pegin_requested);

        let event = RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
            inner: pegin_requested.clone(),
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
        assert!(processor.blockchain.has_observer(&observer_id));
    }

    #[test]
    fn process_removed_event_pegin_accepted_event() {
        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);
        expect_get_comm_info(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        let pegin_requested = dummy_pegin_requested_event();

        add_to_my_committees(processor.global_context.my_committees(), &pegin_requested);

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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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
        expect_get_comm_info(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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

        contracts
            .expect_get_committee()
            .withf(move |inp: &GetCommitteeInput| {
                inp.committee_id == pegin_requested.committeeId.try_into().unwrap()
            })
            .returning(move |_| {
                Ok(GetCommitteeOutput {
                    committee: committee.clone(),
                })
            })
            .times(2); // Called twice: once for build_pegin_request_bitvmx_message and once for get_committee_peer_ids

        contracts
            .expect_my_address()
            .returning(|| types::Address::default())
            .times(1);

        contracts
            .expect_get_member_public_keys()
            .returning(move |_| {
                Ok(GetMemberPublicKeysOutput {
                    public_keys: vec![
                        "0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                            .to_string(), // take key
                        "0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81799"
                            .to_string(), // dispute key
                        "0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f8179a"
                            .to_string(), // communication key at index 2
                    ],
                })
            })
            .times(2); // Called twice for get_committee_peer_ids (2 members)

        // Mock get_committee_communication_data for Setup message
        contracts
            .expect_get_committee_communication_data()
            .withf(move |output| {
                output.committee_id == pegin_requested.committeeId.try_into().unwrap()
            })
            .returning(|_| {
                Ok(GetCommunicationDataOutput {
                    communication_data: vec![
                        P2PAddressParser::addr_to_contracts("/ip4/127.0.0.1/tcp/8080").unwrap(),
                        P2PAddressParser::addr_to_contracts("/ip4/127.0.0.1/tcp/8081").unwrap(),
                    ],
                })
            })
            .times(1);

        let mut broker = MockBrokerClientApi::new();
        expect_bitvmx_subscription_success(&mut broker);
        expect_get_comm_info(&mut broker);

        // Expect PeginRequest message
        broker
            .expect_send()
            .times(1)
            .with(
                eq(BROKER_SERVER_ID),
                function(move |req: &IncomingBitVMXApiMessages| {
                    matches!(
                        req,
                        IncomingBitVMXApiMessages::SetVar(_, var_name, VariableTypes::String(_))
                            if var_name == PEGIN_REQUEST
                    )
                }),
            )
            .returning(|_, _| Ok(true));

        // Expect Setup message
        broker
            .expect_send()
            .times(1)
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &IncomingBitVMXApiMessages| {
                    matches!(
                    req,
                    IncomingBitVMXApiMessages::Setup(_, program_type, p2p_addresses, leader)
                        if program_type == PROGRAM_TYPE_ACCEPT_PEGIN && p2p_addresses.len() == 2 && *leader == 0
                )
                }),
            )
            .returning(|_, _| Ok(true));

        let mock_btc_sig_subflow_factory = MockBtcSigSubFlowFactory::new();

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            contracts.into(),
            broker.into(),
            mock_btc_sig_subflow_factory,
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
        );

        processor
            .blockchain
            .add_observer(pegin_event.confirmations.clone());
        let _ = processor.track_pegin_requested(pegin_flow_id, pegin_event);

        // Simulate receiving CommInfo response
        let dummy_p2p_address = P2PAddress {
            address: "127.0.0.1:8080".to_string(),
            peer_id: PeerId("dummy_peer_id".to_string()),
        };
        let comm_info_event = OutgoingBitVMXApiMessages::CommInfo(dummy_p2p_address);
        let _ = processor.process_new_bitvmx_event(&comm_info_event);

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
        expect_get_comm_info(&mut broker);

        let mut processor = PeginProcessor::new(
            RuntimeSync::new().unwrap(),
            MockRskContractsGatewayApi::new().into(),
            broker.into(),
            MockBtcSigSubFlowFactory::new(),
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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
        expect_get_comm_info(&mut broker);
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
            GlobalContext::new(),
            NativeBridgeVerifier::Dummy,
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

    fn expect_get_comm_info(
        broker: &mut MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
    ) {
        broker
            .expect_send()
            .times(1)
            .withf(|id, msg| {
                *id == BROKER_SERVER_ID && matches!(msg, IncomingBitVMXApiMessages::GetCommInfo())
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
                    txid: TxIdParser::fb_32_to_txid(
                        <[u8; 32]>::from_hex(
                            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        )
                        .unwrap()
                        .into(),
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
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 0.into(),
            },
            requestPeginInfo: RequestPeginTempInfo {
                rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                    .parse::<alloy_primitives::Address>()
                    .expect("Invalid address"),
                btcReimbursementPubKey:
                    "0xc6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
                        .parse::<FixedBytes<32>>()
                        .expect("Failed to parse reimbursement key"),
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
            aggregatedKey: "0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
            members: vec![
                CommitteeMember {
                    memberAddress: leader,
                    role: Role::from(1u8).into(), // Prover
                },
                CommitteeMember {
                    memberAddress: address!("0x0000000000000000000000000000000000000001"),
                    role: Role::from(2u8).into(), // Verifier
                },
            ],
            leaderAddress: leader,
            operatorTakeIndex: U256::from(0u64),
            createdAt: Default::default(),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 0,
            fundingUTXOs: vec![],
        }
    }

    /// helper method to add committee to my_committees for testing
    fn add_to_my_committees(my_committees: &MyCommittees, pegin_requested: &PeginRequested) {
        my_committees.add(
            pegin_requested
                .committeeId
                .try_into()
                .expect("Valid committee id"),
            crate::types::Role::Verifier,
        );
    }
}
