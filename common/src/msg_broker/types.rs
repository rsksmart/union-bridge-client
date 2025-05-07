use crate::types::{RskBlock, RskLog};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum BrokerRequests {
    SubscribeBlocks,
    UnsubscribeBlocks,
    SubscribeLogs(String),
    UnsubscribeLogs(String),
    // TODO(iago) add a limit time for receiving a response?
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BrokerResponses {
    Block(RskBlock),
    Log(RskLog),
}
