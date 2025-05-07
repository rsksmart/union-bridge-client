use crate::types::{BlockNumber, RskBlock, RskLog};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum BrokerRequests {
    SubscribeBlocks,
    UnsubscribeBlocks,
    GetBlock(BlockNumber), // TODO(iago) implement, needed to get the first blocks that might be missing after the trigger event
    SubscribeLogs(String),
    UnsubscribeLogs(String),
    // TODO(iago) add a limit time for receiving a response?
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BrokerResponses {
    Block(RskBlock),
    Log(RskLog),
}
