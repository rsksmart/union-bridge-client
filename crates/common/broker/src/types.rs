use common_core::types::{Address, RskBlockAndUncles, RskLog};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug)]
pub enum ToServer {
    // block-indexer
    SubscribeBlocks,
    UnsubscribeBlocks,

    // log-indexer
    SubscribeLogs(Address),
    UnsubscribeLogs(Address),

    // coordinator -> user-api replies
    MemberFundingInfo(Uuid, MemberFundingInfo),
    BitVmxWalletError(Uuid, String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MemberFundingInfo {
    pub bitcoin_address: String,
    pub rsk_address: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Block payload is intentionally larger than other broker messages
pub enum FromServer {
    // Indexers
    Block(RskBlockAndUncles),
    Log(RskLog),

    // User API
    UserRequest(Value),
    MemberRequest(Uuid),

    // fake bitvmx incoming messages
    RegisterPegout(Value),
}
