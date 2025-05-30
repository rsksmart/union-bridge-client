use crate::types::{Address, BlockNumber, RskBlockAndUncles, RskLog};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum BrokerRequests {
    SubscribeBlocks,
    UnsubscribeBlocks,
    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - implement block retrieval, it will be needed to get the first blocks that might be missing after the trigger event (event received after block)
    GetBlock(BlockNumber),
    SubscribeLogs(Address),
    UnsubscribeLogs(Address),
    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - add a limit time for receiving a response?
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BrokerResponses {
    Block(RskBlockAndUncles),
    Log(RskLog),
}
