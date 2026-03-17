use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Address, RskBlockAndUncles, RskLog};

// TODO(UB-213): consider response type / timeout handling

#[derive(Serialize, Deserialize, Debug)]
pub enum ToServer {
    // TODO(UB-132): add a time limit for receiving a response?

    // block-indexer
    SubscribeBlocks,
    UnsubscribeBlocks,

    // log-indexer
    SubscribeLogs(Address),
    UnsubscribeLogs(Address),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Block payload is intentionally larger than other broker messages
pub enum FromServer {
    // Indexers
    Block(RskBlockAndUncles),
    Log(RskLog),

    // User API
    UserRequest(Value),
    MemberRequest,

    // fake bitvmx incoming messages
    RegisterPegout(Value),
}
