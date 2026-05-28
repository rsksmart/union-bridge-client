use anyhow::{Context, Result};
use bitcoin::PublicKey;
use bitcoin::key::Parity::Even;
use bitcoin::secp256k1::XOnlyPublicKey;
use common::msg_broker::bitvmx_types::AdvanceFundsRegistered;
use common::types::{Address, CommitteeId, Hash256, TxIdParser};
use uuid::Uuid;

use crate::types::OperatorTakeTriggeredEvent;

#[derive(Debug, Clone)]
pub(crate) struct OperatorTakeTriggerData {
    pub(crate) pegout_txid: Hash256,
    pub(crate) pegout_id: Hash256,
    pub(crate) committee_id: CommitteeId,
    pub(crate) slot_id: u64,
    pub(crate) slot_index: usize,
    pub(crate) request_pegout_tx_hash: Option<String>,
    pub(crate) user_pubkey: PublicKey,
    pub(crate) take_operator_address: Address,
    pub(crate) operator_take_pubkey: PublicKey,
}

impl OperatorTakeTriggerData {
    pub(crate) fn try_from_event(
        event: &OperatorTakeTriggeredEvent,
        request_pegout_tx_hash: Option<String>,
    ) -> Result<Self> {
        let inner = &event.inner;
        let pegout_txid = Hash256::from(inner.pegoutTxid);
        let pegout_id = Hash256::from(inner.pegoutInfo.pegoutId);
        let committee_id = CommitteeId::from(inner.pegoutInfo.committeeId);
        let slot_id = inner.streamPosition.slotId;
        let slot_index = usize::try_from(slot_id)
            .context("Failed to convert slot id from event into usize for slot index")?;
        let user_pubkey = PublicKey::from_slice(inner.pegoutInfo.userPubKey.as_ref())?;
        let take_operator_address = Address::from(inner.pegoutInfo.takeOperatorAddress);
        let operator_take_pubkey =
            xonly_to_compressed_pubkey(inner.pegoutInfo.operatorTakePubKey.as_ref())?;
        Ok(Self {
            pegout_txid,
            pegout_id,
            committee_id,
            slot_id,
            slot_index,
            request_pegout_tx_hash,
            user_pubkey,
            take_operator_address,
            operator_take_pubkey,
        })
    }
}

fn xonly_to_compressed_pubkey(bytes: &[u8]) -> Result<PublicKey> {
    let xonly =
        XOnlyPublicKey::from_slice(bytes).context("Failed to parse x-only public key bytes")?;
    let secp_pubkey = xonly.public_key(Even);
    Ok(PublicKey::new(secp_pubkey))
}

/// Translate the on-chain `AdvanceFundsRegistered` solidity event into the
/// runtime/BitVMX domain type. Lives here (next to `OperatorTakeTriggerData`)
/// so the processor only routes events — it doesn't assemble flow-domain
/// types.
pub(crate) fn advance_funds_registered_from_event(
    event: &union_contracts::bindings::pegout_manager::PegoutManager::AdvanceFundsRegistered,
) -> Result<AdvanceFundsRegistered> {
    let committee_id = Uuid::from_u128(event.committeeId);
    let slot_index =
        usize::try_from(event.streamInfo.slotId).context("Failed to convert slotId to usize")?;
    let txid = TxIdParser::fb_32_to_txid(event.txid);
    let pegout_id = event.pegoutId.as_slice().to_vec();
    let operator_pubkey = xonly_to_compressed_pubkey(event.operatorTakePubKey.as_slice())?;

    Ok(AdvanceFundsRegistered { committee_id, slot_index, txid, pegout_id, operator_pubkey })
}
