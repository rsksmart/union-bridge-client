use crate::types::{Role, Utxo};
use common::types::StreamId;
use serde::Deserialize;
// TODO create types mod and move this and types.rs (renamed to rsk_events.rs) there

#[derive(Clone, Debug, Deserialize)]
pub struct ApplyToStream {
    pub stream_id: StreamId, // Matches StreamDenomination in the contract
    pub role: Role,
    pub funding_utxo: Utxo,
    pub speed_up_utxo: Utxo,
}
