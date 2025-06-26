use crate::types::{Address, RskBlockAndUncles, RskLog};
use bitvmx_client::types::IncomingBitVMXApiMessages;
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

    // real BitVMX API messages
    ToBitVMX(IncomingBitVMXApiMessages),

    // fake bitvmx outgoing messages
    SubscribeMockedBitVMX,
    UnsubscribeMockedBitVMX,
    TemporaryPegInAddressMockedBitVMX(Value),
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub enum FromServer {
    Block(RskBlockAndUncles),
    Log(RskLog),

    // fake bitvmx incoming messages
    GetTemporaryPegInAddress(Value),
}
