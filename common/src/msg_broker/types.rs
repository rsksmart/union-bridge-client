use crate::types::{Address, RskBlockAndUncles, RskLog};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
pub enum ToServer {
    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - add a limit time for receiving a response?

    // block-indexer
    SubscribeBlocks,
    UnsubscribeBlocks,

    // log-indexer
    SubscribeLogs(Address),
    UnsubscribeLogs(Address),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum FromServer {
    // Indexers
    Block(RskBlockAndUncles),
    Log(RskLog),

    // User API
    UserApplyStream(Value), // TODO(iago) get TransactionDispatcher destination type for now (while not moved to coomon)

    // fake bitvmx incoming messages
    RegisterPegout(Value),
}
