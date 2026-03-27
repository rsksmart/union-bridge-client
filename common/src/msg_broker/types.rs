use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Address, RskBlockAndUncles, RskLog};

// TODO(Jira) UB-213

#[derive(Serialize, Deserialize, Debug)]
pub enum ToServer {
    // TODO(Jira) UB-132 - add a limit time for receiving a response?

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
