use anyhow::{Context, Result};
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::{
    BitVmxProtocolId, IncomingBitVMXApiMessages, PartialUtxo, dispute_core_protocol_id,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::types::CommitteeId;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use union_contracts::bindings::committee_registry::CommitteeRegistry::Committee;
use uuid::Uuid;

use crate::types::MemberOfCommittee;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FundingUtxos {
    pub speedup: PartialUtxo,
    pub protocol_funding: PartialUtxo,
    pub advance_funds: PartialUtxo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommitteeData {
    pub committee_id: CommitteeId,
    pub committee: Committee,
    pub members: Vec<MemberOfCommittee>,
}

impl CommitteeData {
    /// Converts the `CommitteeId` to a Uuid.
    /// This is a common operation that's repeated multiple times across the codebase.
    pub(super) fn committee_uuid(&self) -> Uuid {
        Uuid::from_u128(*self.committee_id)
    }

    /// Gets the dispute core protocol ID for a member by their take key.
    /// This is a common operation that's repeated multiple times across the codebase.
    pub(super) fn get_dispute_core_pid_for_key(&self, pubkey: &PublicKey) -> BitVmxProtocolId {
        dispute_core_protocol_id(self.committee_uuid(), pubkey)
    }

    /// Gets the dispute core protocol ID for a member by their index.
    /// This is a common operation that's repeated multiple times across the codebase.
    pub(super) fn get_dispute_core_pid_for_index(
        &self,
        member_index: usize,
    ) -> Result<BitVmxProtocolId> {
        let member = self
            .members
            .get(member_index)
            .context(format!("Member index {member_index} out of bounds"))?;
        Ok(self.get_dispute_core_pid_for_key(&member.take_key))
    }
}

/// Sends a message to `BitVMX` broker with proper error handling and logging.
/// Returns Result to allow error propagation.
pub(super) fn send_bitvmx_msg<BC: BitVmxBrokerClientApi>(
    broker_client: &BC,
    msg: IncomingBitVMXApiMessages,
) -> Result<()> {
    debug!("Sending to BitVMX: {msg:?}");

    broker_client
        .send(msg)
        .map(|_| ())
        .map_err(|e| {
            error!("Failed to send msg to BitVMX: {e:?}");
            anyhow::Error::from(e)
        })
        .context("Failed to send message to BitVMX broker")
}
