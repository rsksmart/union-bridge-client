use anyhow::Context;
use common::msg_broker::bitvmx_types::ParticipantRole;
use common::types::StreamId;
use serde::{Deserialize, Serialize};
use union_contracts::bindings::committee_registry::CommitteeRegistry::UTXO;
// TODO create types mod and move this and types.rs (renamed to rsk_events.rs) there

#[derive(Clone, Debug, Deserialize)]
pub struct ApplyToStream {
    pub stream_id: StreamId, // Matches StreamDenomination in the contract
    pub role: Role,
    pub utxo: Utxo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role {
    Prover,
    Verifier,
}

impl TryFrom<Utxo> for UTXO {
    type Error = anyhow::Error;

    fn try_from(utxo: Utxo) -> Result<Self, Self::Error> {
        Ok(UTXO {
            txid: utxo
                .txid
                .parse()
                .context("Failed to parse String txid to FixedBytes<32>")?,
            outputIndex: utxo.vout,
            amount: utxo.value,
        })
    }
}

impl From<Role> for ParticipantRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Prover => ParticipantRole::Prover,
            Role::Verifier => ParticipantRole::Verifier,
        }
    }
}

impl From<Role> for u8 {
    fn from(role: Role) -> Self {
        match role {
            Role::Prover => 1,
            Role::Verifier => 2,
        }
    }
}
