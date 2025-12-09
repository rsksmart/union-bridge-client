use common::msg_broker::bitvmx_types::ParticipantRole;
use common::types::StreamId;
use serde::{Deserialize, Serialize};

use crate::types::Utxo;
// TODO create types mod and move this and types.rs (renamed to rsk_events.rs) there

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyToStream {
    pub stream_id: StreamId, // Matches StreamDenomination in the contract
    pub role: ParticipantRole,
    pub funding_utxo: Utxo,
    pub speed_up_utxo: Utxo,
}
