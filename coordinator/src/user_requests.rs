use common::msg_broker::bitvmx_types::{PartialUtxo, ParticipantRole};
use serde::Deserialize;

// TODO create types mod and move this and types.rs (renamed to rsk_events.rs) there

#[derive(Clone, Debug, Deserialize)]
pub struct ApplyToStream {
    pub stream_id: u8,
    pub role: ParticipantRole,
    pub utxo: Vec<PartialUtxo>, // 3: 1) speed up, 2) funding for initial tx of the dispute core, 3) for funds advancement on pegout
}
