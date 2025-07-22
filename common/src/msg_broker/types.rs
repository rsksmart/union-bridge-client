use serde::{Deserialize, Serialize};

use crate::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use crate::types::{Address, RskBlockAndUncles, RskLog};

#[derive(Serialize, Deserialize, Debug)]
pub enum ToServer {
    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - add a limit time for receiving a response?

    // block-indexer
    SubscribeBlocks,
    UnsubscribeBlocks,

    // log-indexer
    SubscribeLogs(Address),
    UnsubscribeLogs(Address),

    // real BitVMX API messages
    ToBitVMX(IncomingBitVMXApiMessages),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum FromServer {
    Block(RskBlockAndUncles),
    Log(RskLog),

    // real BitVMX incoming messages
    FromBitVMX(OutgoingBitVMXApiMessages),
}
