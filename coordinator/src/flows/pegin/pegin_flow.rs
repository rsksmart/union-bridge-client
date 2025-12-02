use crate::flows::common::{COMM_KEY_INDEX, build_communication_data};
use crate::flows::pegin::utils::{get_accept_pegin_pid, get_temp_pegin_pid};
use crate::store::{CoordinatorStoreApi, StoreKey};

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::{
    PublicKey, Txid,
    secp256k1::{Parity::Even, XOnlyPublicKey},
};
use common::{
    msg_broker::{
        bitvmx_types::{
            ACCEPT_PEGIN_TX, BtcTxSPVProof, IncomingBitVMXApiMessages, P2PAddress, PeerId,
            PeginAcceptedMessage, TransactionStatus, VariableTypes,
        },
        broker::BitVmxBrokerClientApi,
    },
    runtime_sync::RuntimeSync,
    types::{CommitteeId, TxIdParser},
};
use log::{debug, info, trace};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use transaction_dispatcher::{
    rsk_gateway::RskContractsGatewayApi,
    types::{
        GetCommitteeInput, GetCommitteeOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
        P2PAddressParser, RequestPeginInput,
    },
};
use union_contracts::bindings::peg_manager::PegManager::{PeginAccepted, PeginRequested};
use uuid::Uuid;

const PEGIN_REQUEST: &str = "pegin_request";
const PEGIN_ACCEPTED_VAR_NAME: &str = "PeginAccepted";
const PROGRAM_TYPE_ACCEPT_PEGIN: &str = "accept_pegin";

/// Steps for the pegin state machine flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Steps {
    // Initial state when Bitcoin pegin transaction is found
    PeginTransactionFound,
    // Request SPV proof for the pegin request (to call requestPegin)
    RequestPeginSpvProof,
    // After RSK PeginRequested event confirmed (contains actual event data)
    PeginRequested,
    // Get communication info from BitVMX
    GetCommInfo,
    // Send pegin request to BitVMX and setup
    PreparePeginSetup,
    // Add operator take tx hash to contracts after BitVMX accepts
    AddOperatorTakeHash,
    // Signature flow is executed between these steps (outside the flow)
    DispatchTransaction,
    // Confirm accept pegin transaction
    ConfirmAcceptPeginTransaction,
    // Request SPV proof for the accept pegin (to call acceptPegin)
    RequestAcceptPeginSpvProof,
    // Accept the pegin on RSK
    AcceptPegin,
    // Final state
    Done,
}

impl Default for Steps {
    fn default() -> Self {
        Steps::PeginTransactionFound
    }
}

/// Data passed between steps in the pegin flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepData {
    // Initial Bitcoin transaction found
    PeginTransactionFound,
    // SPV proof for request pegin
    RequestPeginSpvProof(BtcTxSPVProof),
    // Pegin requested
    PeginRequested(PeginRequested),
    // Communication info
    CommInfo(P2PAddress),
    // BitVMX pegin accepted
    BitvmxPeginAccepted(PeginAcceptedMessage),
    // Operator take hash added
    OperatorTakeHashAdded,
    // Dispatch accept pegin transaction
    DispatchAcceptPeginTransaction,
    // Accept pegin transaction confirmed
    AcceptPeginTransactionConfirmed(TransactionStatus),
    // SPV proof for accept pegin
    AcceptPeginSpvProof(BtcTxSPVProof),
    // Pegin accepted
    PeginAccepted(PeginAccepted),
}

/// Data structure used to send pegin request information to BitVMX
#[derive(Debug, Clone, Serialize)]
pub struct PeginRequestMessage {
    pub txid: Txid,
    pub amount: u64,
    pub accept_pegin_sighash: Vec<u8>,
    pub take_aggregated_key: PublicKey,
    pub operator_indexes: Vec<usize>,
    pub slot_index: u64,
    pub committee_id: Uuid,
    pub rootstock_address: String,
    pub reimbursement_pubkey: PublicKey,
}

/// Context for the pegin flow state machine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowContext {
    pub flow_id: Uuid,
    pub step: Steps,

    // ID tracking for migration
    pub temp_flow_id: Option<Uuid>, // Temporary ID (Bitcoin txid-based)
    pub official_flow_id: Option<Uuid>, // Official ID (committee+slot based)

    pub request_pegin_btc_tx_id: Option<Txid>,
    pub request_pegin_btc_tx_status: Option<TransactionStatus>,
    pub request_pegin_spv_proof: Option<BtcTxSPVProof>,
    pub pegin_requested: Option<PeginRequested>,
    pub my_p2p_address: Option<P2PAddress>,
    pub committee_output: Option<GetCommitteeOutput>,
    pub bitvmx_pegin_accepted: Option<PeginAcceptedMessage>,
    pub accept_pegin_spv_proof: Option<BtcTxSPVProof>,
    pub accept_pegin_tx_status: Option<TransactionStatus>,
    pub pegin_accepted: Option<PeginAccepted>,
}

/// Serializable state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub flow_id: Uuid,
    pub ctx: FlowContext,
}

/// State machine for handling pegin flow
pub struct PeginFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    state: State,
    store: Rc<S>,
}

impl<CG, BC, S> PeginFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    /// Create a new pegin flow from PeginTransactionFound
    pub fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        btc_tx_id: Txid,
        store: Rc<S>,
    ) -> Self {
        let temp_flow_id = get_temp_pegin_pid(btc_tx_id);

        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: State {
                flow_id: temp_flow_id,
                ctx: FlowContext {
                    flow_id: temp_flow_id,
                    step: Steps::PeginTransactionFound,
                    temp_flow_id: Some(temp_flow_id),
                    official_flow_id: None,
                    request_pegin_btc_tx_id: Some(btc_tx_id),
                    request_pegin_btc_tx_status: None,
                    request_pegin_spv_proof: None,
                    pegin_requested: None,
                    my_p2p_address: None,
                    committee_output: None,
                    bitvmx_pegin_accepted: None,
                    accept_pegin_spv_proof: None,
                    accept_pegin_tx_status: None,
                    pegin_accepted: None,
                },
            },
            store,
        }
    }

    pub fn from_saved_state(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        state: State,
        store: Rc<S>,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state,
            store,
        }
    }

    fn persist_state(&self) -> Result<()> {
        debug!(
            "PeginFlow {}: Persisting state for step: {:?}",
            self.state.flow_id, self.state.ctx.step
        );
        self.store
            .save_flow(StoreKey::PeginFlow(self.state.flow_id), self.state.clone())
    }

    /// Start the next step and log the transition
    pub fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.ctx.step;
        self.state.ctx.step = next_step;

        debug!(
            "PeginFlow {}: {} -> {}",
            self.state.flow_id,
            format_step(previous_step),
            format_step(next_step)
        );

        // Execute the entry action for the new state
        match next_step {
            Steps::PeginTransactionFound => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::RequestPeginSpvProof => {
                info!(
                    "Requesting SPV proof for pegin request for flow_id: {}",
                    self.state.flow_id
                );
                self.request_pegin_spv_proof()?;
            }
            Steps::PeginRequested => {
                // This step will be reached after PeginRequested event is confirmed
                info!(
                    "PeginRequested event confirmed for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::GetCommInfo => {
                self.migrate_to_official_flow_id()?;
                self.request_bitvmx_comm_info()?;
            }
            Steps::PreparePeginSetup => {
                self.prepare_pegin_setup()?;
            }
            Steps::AddOperatorTakeHash => {
                self.add_operator_take_hash()?;
            }
            Steps::DispatchTransaction => {
                info!(
                    "Waiting for signatures to be ready to dispatch transaction for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::ConfirmAcceptPeginTransaction => {
                self.dispatch_transaction()?;
                info!(
                    "Waiting for transaction confirmations for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id,
                    self.get_accept_pegin_txid()
                );
            }
            Steps::RequestAcceptPeginSpvProof => {
                info!(
                    "Requesting SPV proof for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id,
                    self.get_accept_pegin_txid()
                );
                self.request_spv_proof()?;
            }
            Steps::AcceptPegin => {
                info!("Accepting pegin for flow_id: {}", self.state.flow_id);
                let spv_proof = self
                    .state
                    .ctx
                    .accept_pegin_spv_proof
                    .clone()
                    .ok_or_else(|| {
                        anyhow!(
                            "SPV proof not available for pegin acceptance - flow_id {}",
                            self.state.flow_id
                        )
                    })?;
                self.accept_pegin(spv_proof)?;
            }
            Steps::Done => {
                self.send_pegin_accepted_to_bitvmx()?;
                info!("PeginFlow {}: Done", self.state.flow_id);
            }
        }

        // Persist state after successful step completion
        self.persist_state()?;

        Ok(())
    }

    /// Complete the current step with data and advance to the next
    pub fn complete_step(&mut self, data: StepData) -> Result<()> {
        let current_step = self.state.ctx.step;

        info!(
            "PeginFlow {}: Completing step {} with data: {:?}",
            self.state.flow_id,
            format_step(current_step),
            data
        );

        // Process data and determine next state
        let next_step = self.process_step_data(current_step, &data)?;

        // Transition to the next state
        self.start_step(next_step)?;

        Ok(())
    }

    /// Process the current step data and determine the next state
    fn process_step_data(&mut self, current_step: Steps, data: &StepData) -> Result<Steps> {
        match (current_step, data) {
            (Steps::PeginTransactionFound, StepData::PeginTransactionFound) => {
                Ok(Steps::RequestPeginSpvProof)
            }
            (Steps::RequestPeginSpvProof, StepData::RequestPeginSpvProof(spv_proof)) => {
                self.state.ctx.request_pegin_spv_proof = Some(spv_proof.clone());
                Ok(Steps::PeginRequested)
            }
            (Steps::PeginRequested, StepData::PeginRequested(pegin_requested)) => {
                self.state.ctx.pegin_requested = Some(pegin_requested.clone());
                Ok(Steps::GetCommInfo)
            }
            (Steps::GetCommInfo, StepData::CommInfo(comm_info)) => {
                self.state.ctx.my_p2p_address = Some(comm_info.clone());
                Ok(Steps::PreparePeginSetup)
            }
            (Steps::PreparePeginSetup, StepData::BitvmxPeginAccepted(accepted)) => {
                self.state.ctx.bitvmx_pegin_accepted = Some(accepted.clone());
                Ok(Steps::AddOperatorTakeHash)
            }
            (Steps::AddOperatorTakeHash, StepData::OperatorTakeHashAdded) => {
                Ok(Steps::DispatchTransaction)
            }
            (Steps::DispatchTransaction, StepData::DispatchAcceptPeginTransaction) => {
                Ok(Steps::ConfirmAcceptPeginTransaction)
            }
            (
                Steps::ConfirmAcceptPeginTransaction,
                StepData::AcceptPeginTransactionConfirmed(tx_status),
            ) => {
                info!(
                    "Transaction confirmed for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id, tx_status.tx_id
                );
                trace!("Transaction status data: {:?}", tx_status);
                let expected_tx_id = self
                    .get_accept_pegin_txid()
                    .ok_or_else(|| anyhow!("Expected accept pegin txid not found"))?;
                if tx_status.tx_id != expected_tx_id {
                    bail!(
                        "Transaction status txId mismatch: got {:?}, expected {:?}",
                        tx_status.tx_id,
                        expected_tx_id
                    );
                }
                self.state.ctx.accept_pegin_tx_status = Some(tx_status.clone());
                Ok(Steps::RequestAcceptPeginSpvProof)
            }
            (Steps::RequestAcceptPeginSpvProof, StepData::AcceptPeginSpvProof(spv_proof)) => {
                info!("Received SPV proof for flow_id: {}", self.state.flow_id);
                trace!("SPV Proof data: {:?}", spv_proof);
                self.state.ctx.accept_pegin_spv_proof = Some(spv_proof.clone());
                Ok(Steps::AcceptPegin)
            }
            (Steps::AcceptPegin, StepData::PeginAccepted(pegin_accepted)) => {
                info!(
                    "Pegin accepted successfully for flow_id: {}",
                    self.state.flow_id
                );
                trace!("PeginAccepted data: {:?}", pegin_accepted);
                self.state.ctx.pegin_accepted = Some(pegin_accepted.clone());
                Ok(Steps::Done)
            }
            _ => Err(anyhow!(
                "Invalid state transition: {:?} with data {:?}",
                current_step,
                data
            )),
        }
    }

    fn prepare_pegin_setup(&mut self) -> Result<()> {
        info!(
            "Preparing pegin setup for BitVMX with flow_id: {}",
            self.state.flow_id
        );

        let pegin_requested = self
            .state
            .ctx
            .pegin_requested
            .as_ref()
            .ok_or_else(|| anyhow!("PeginRequested data not available"))?;
        let committee_id: CommitteeId = pegin_requested.committeeId.try_into()?;

        self.send_pegin_request_to_bitvmx(&committee_id)?;
        self.send_setup_to_bitvmx(&committee_id)?;
        Ok(())
    }

    fn send_pegin_request_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!(
            "Sending pegin request to BitVMX with flow_id: {}",
            self.state.flow_id
        );

        let committee_output = self.get_committee_output(committee_id.clone())?;
        self.state.ctx.committee_output = Some(committee_output.clone());

        let pegin_requested = self
            .state
            .ctx
            .pegin_requested
            .as_ref()
            .ok_or_else(|| anyhow!("PeginRequested data not available"))?;
        let pegin_request = self.build_pegin_request_message(pegin_requested, &committee_output)?;

        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.flow_id,
            PEGIN_REQUEST.to_string(),
            VariableTypes::String(serde_json::to_string(&pegin_request)?),
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    fn send_setup_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!(
            "Sending setup to BitVMX with flow_id: {}",
            self.state.flow_id
        );

        let committee_addresses = self.get_committee_addresses(committee_id)?;
        let committee_peer_ids = self.get_committee_peer_ids(committee_id)?;

        let p2p_addresses = build_communication_data(
            self.state
                .ctx
                .my_p2p_address
                .as_ref()
                .ok_or_else(|| anyhow!("P2P address not available for setup"))?
                .address
                .clone(),
            committee_addresses,
            committee_peer_ids,
        )?;

        let msg = IncomingBitVMXApiMessages::Setup(
            self.state.flow_id,
            PROGRAM_TYPE_ACCEPT_PEGIN.to_string(),
            p2p_addresses,
            0, // No leader
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    fn add_operator_take_hash(&self) -> Result<()> {
        let pegin_accepted = self
            .state
            .ctx
            .bitvmx_pegin_accepted
            .as_ref()
            .ok_or_else(|| anyhow!("BitVMX pegin accepted message not found"))?;

        debug!(
            "Adding operator take tx hash for flow_id: {}, accept_pegin_txid: {}",
            self.state.flow_id, pegin_accepted.accept_pegin_txid
        );

        let input = transaction_dispatcher::types::AddOperatorTakeTxHashInput {
            accept_pegin_tx_hash: pegin_accepted.accept_pegin_txid,
            take_tx_hash: pegin_accepted.operator_take_sighash.clone(),
        };

        self.rt_sync
            .run(async { self.contracts.add_operator_take_tx_hash(input).await })?;

        Ok(())
    }

    fn dispatch_transaction(&self) -> Result<()> {
        info!(
            "Dispatching transaction {} for flow_id: {}",
            ACCEPT_PEGIN_TX, self.state.flow_id
        );
        let msg = IncomingBitVMXApiMessages::DispatchTransactionName(
            self.state.flow_id,
            ACCEPT_PEGIN_TX.to_string(),
        );
        self.send_bitvmx_msg(msg)?;
        Ok(())
    }

    fn accept_pegin(&self, spv_proof: BtcTxSPVProof) -> Result<()> {
        debug!("Accepting pegin with SPV proof: {:?}", spv_proof);

        let input: RequestPeginInput = spv_proof.into();

        self.rt_sync
            .run(async { self.contracts.accept_pegin(input).await })
            .context("Failed to accept pegin with provided SPV proof")?;

        Ok(())
    }

    fn send_pegin_accepted_to_bitvmx(&self) -> Result<()> {
        let pegin_accepted = self
            .state
            .ctx
            .pegin_accepted
            .as_ref()
            .ok_or_else(|| anyhow!("PeginAccepted data not available"))?;

        debug!(
            "Notifying pegin accepted to BitVMX with flow_id: {}",
            self.state.flow_id
        );
        let data = serde_json::to_string(&pegin_accepted)?;
        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.flow_id,
            PEGIN_ACCEPTED_VAR_NAME.to_string(),
            VariableTypes::String(data),
        );

        self.send_bitvmx_msg(msg)
    }

    fn request_pegin_spv_proof(&self) -> Result<()> {
        let btc_tx_id = self
            .state
            .ctx
            .request_pegin_btc_tx_id
            .ok_or_else(|| anyhow!("Bitcoin transaction ID not available"))?;

        info!(
            "Requesting SPV proof for Bitcoin transaction: {}",
            btc_tx_id
        );
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(btc_tx_id))?;
        Ok(())
    }

    fn migrate_to_official_flow_id(&mut self) -> Result<()> {
        let pegin_requested = self
            .state
            .ctx
            .pegin_requested
            .as_ref()
            .ok_or_else(|| anyhow!("PeginRequested data not available for migration"))?;

        // Calculate official flow ID from committee and slot information
        let committee_id: CommitteeId = pegin_requested.committeeId.try_into()?;
        let committee_uuid: Uuid = Uuid::from_u128(*committee_id);
        let official_flow_id = get_accept_pegin_pid(
            committee_uuid,
            pegin_requested.streamPosition.slotId as usize,
        )?;

        info!(
            "Migrating flow from temp ID {} to official ID {}",
            self.state.flow_id, official_flow_id
        );

        // Update flow context with official ID
        self.state.ctx.official_flow_id = Some(official_flow_id);
        self.state.flow_id = official_flow_id;

        Ok(())
    }

    fn build_pegin_request_message(
        &self,
        event: &PeginRequested,
        committee_output: &GetCommitteeOutput,
    ) -> Result<PeginRequestMessage> {
        debug!("Building PeginRequestMessage for BitVMX from PeginRequested event");

        let committee_id = Uuid::from_u128(event.committeeId.try_into()?);
        let operator_indexes = self.build_operator_indexes(committee_output)?;
        let slot_index = event.streamPosition.slotId;

        let checksum_address = event
            .requestPeginInfo
            .rskDestinationAddress
            .to_checksum(None);
        let rootstock_address = checksum_address
            .get(2..)
            .ok_or_else(|| anyhow!("RSK address checksum too short"))?
            .to_string();

        let accept_pegin_sighash = event.acceptPeginSignatureMessage.to_vec();
        let take_aggregated_key = self.build_take_aggregated_key(committee_output)?;
        let reimbursement_pubkey = self.build_reimbursement_pubkey(event)?;
        let txid = TxIdParser::fb_32_to_txid(event.requestPeginTxid);

        Ok(PeginRequestMessage {
            txid,
            amount: event.prevoutData.value,
            accept_pegin_sighash,
            take_aggregated_key,
            operator_indexes,
            slot_index,
            committee_id,
            rootstock_address,
            reimbursement_pubkey,
        })
    }

    fn build_operator_indexes(
        &self,
        committee_response: &GetCommitteeOutput,
    ) -> Result<Vec<usize>> {
        let operator_role: u8 = crate::types::Role::Prover.into();
        let mut operator_indexes = Vec::new();

        for (i, member) in committee_response.committee.members.iter().enumerate() {
            if member.role == operator_role {
                operator_indexes.push(i);
            }
        }

        Ok(operator_indexes)
    }

    fn build_take_aggregated_key(
        &self,
        committee_response: &GetCommitteeOutput,
    ) -> Result<PublicKey> {
        PublicKey::from_slice(&committee_response.committee.aggregatedKey)
            .context("Failed to parse aggregated public key from committee")
    }

    fn build_reimbursement_pubkey(&self, event: &PeginRequested) -> Result<PublicKey> {
        let reimbursement_xonly_key =
            XOnlyPublicKey::from_slice(event.requestPeginInfo.btcReimbursementPubKey.as_slice())
                .context("Failed to parse reimbursement public key from pegin event")?;
        let reimbursement_secp_key = reimbursement_xonly_key.public_key(Even);
        Ok(PublicKey::new(reimbursement_secp_key))
    }

    fn get_committee_output(&mut self, committee_id: CommitteeId) -> Result<GetCommitteeOutput> {
        let committee_response = self.rt_sync.run(async {
            self.contracts
                .get_committee(GetCommitteeInput { committee_id })
                .await
        })?;
        Ok(committee_response)
    }

    fn get_committee_addresses(&self, committee_id: &CommitteeId) -> Result<Vec<String>> {
        let input = GetCommunicationDataInput {
            committee_id: committee_id.clone(),
            member_address: self.contracts.my_address().into(),
        };
        let communication_data_response = self
            .rt_sync
            .run(async { self.contracts.get_committee_communication_data(input).await })?;

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

    fn get_committee_peer_ids(&self, committee_id: &CommitteeId) -> Result<Vec<PeerId>> {
        let committee_input = GetCommitteeInput {
            committee_id: committee_id.clone(),
        };
        let committee_response = self
            .rt_sync
            .run(async { self.contracts.get_committee(committee_input).await })?;

        let mut peer_ids = Vec::new();

        for member in committee_response.committee.members {
            let keys_input = GetMemberPublicKeysInput {
                member_address: member.memberAddress,
            };

            let keys_response = self
                .rt_sync
                .run(async { self.contracts.get_member_public_keys(keys_input).await })?;

            let key_str = keys_response
                .public_keys
                .get(COMM_KEY_INDEX)
                .with_context(|| {
                    format!(
                        "Communication key not found for member {}",
                        member.memberAddress
                    )
                })?;

            debug!(
                "Member PeerId: address={}, peer_id={:?}",
                member.memberAddress, key_str
            );
            peer_ids.push(PeerId(key_str.to_string()));
        }

        Ok(peer_ids)
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        info!(
            "Requesting BitVMX comm info for flow_id: {}",
            self.state.flow_id
        );
        let req_id = Uuid::new_v4();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(req_id))
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("Sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    pub fn request_transaction_status(&self) -> Result<()> {
        let tx_id = self
            .get_accept_pegin_txid()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;
        info!(
            "Requesting transaction status for flow_id: {} and tx_id: {:?}",
            self.state.flow_id, tx_id
        );
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(
            self.state.flow_id,
            tx_id,
        ))?;
        Ok(())
    }

    pub fn request_spv_proof(&self) -> Result<()> {
        let tx_id = self
            .get_accept_pegin_txid()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))?;
        Ok(())
    }

    pub fn get_accept_pegin_txid(&self) -> Option<Txid> {
        self.state
            .ctx
            .bitvmx_pegin_accepted
            .as_ref()
            .map(|accepted| accepted.accept_pegin_txid)
    }

    /// Check if the flow is completed
    pub fn is_done(&self) -> bool {
        self.state.ctx.step == Steps::Done
    }

    /// Get the internal ID of the flow
    pub fn flow_id(&self) -> Uuid {
        self.state.flow_id
    }

    /// Get the current step
    pub fn current_step(&self) -> Steps {
        self.state.ctx.step
    }

    /// Get the state for debugging
    pub fn get_state(&self) -> &State {
        &self.state
    }

    /// Get the BitVMX pegin accepted message if available
    pub fn get_bitvmx_pegin_accepted(&self) -> Option<&PeginAcceptedMessage> {
        self.state.ctx.bitvmx_pegin_accepted.as_ref()
    }
}

/// Helper function to format step names for logging
fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::PeginTransactionFound => "PeginTransactionFound",
        Steps::RequestPeginSpvProof => "RequestPeginSpvProof",
        Steps::PeginRequested => "PeginRequested",
        Steps::GetCommInfo => "GetCommInfo",
        Steps::PreparePeginSetup => "PreparePeginSetup",
        Steps::AddOperatorTakeHash => "AddOperatorTakeHash",
        Steps::DispatchTransaction => "DispatchTransaction",
        Steps::ConfirmAcceptPeginTransaction => "ConfirmAcceptPeginTransaction",
        Steps::RequestAcceptPeginSpvProof => "RequestAcceptPeginSpvProof",
        Steps::AcceptPegin => "AcceptPegin",
        Steps::Done => "Done",
    }
}
