use crate::types::{Address, BlockHash, BlockNumber, ContractInfo, RskBlock, RskEvent, RskLog};
use anyhow::Result;
use thiserror::Error;

use mockall::automock;

#[derive(Debug)]
pub enum BlockNumRef {
    Latest,
    Number(u64),
}

#[automock]
pub trait RskSubscription<T> {
    fn next(&mut self) -> Result<T, RskSubscriptionError>;
    fn unsubscribe(&self) -> Result<()>;
}

#[derive(Debug, PartialEq, Clone)]
pub struct RskSubscriptionFilter {
    pub addresses: Vec<Address>,
    pub topics: Vec<String>,
    pub from_block: Option<BlockNumber>,
}

impl RskSubscriptionFilter {
    pub fn new(
        addresses: Vec<Address>,
        topics: Vec<String>,
        from_block: Option<BlockNumber>,
    ) -> Self {
        Self {
            addresses,
            topics,
            from_block,
        }
    }
}

#[automock(
    type BlockSubscription = MockRskSubscription<RskBlock>;
    type LogSubscription = MockRskSubscription<RskLog>;
)]
pub trait RskProvider {
    type BlockSubscription: RskSubscription<RskBlock>;
    type LogSubscription: RskSubscription<RskLog>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSubscription>;
    fn subscribe_logs(&self, filter: RskSubscriptionFilter) -> Result<Self::LogSubscription>;
    fn get_block_by_hash(&self, hash: BlockHash) -> Result<Option<RskBlock>>;
    fn get_block_by_number(&self, num: BlockNumber) -> Result<Option<RskBlock>>;
    fn get_uncle_by_hash_and_index(&self, hash: BlockHash, index: u64) -> Result<Option<RskBlock>>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn get_logs(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        addrs: &[Address],
    ) -> Result<Vec<RskLog>>;
    fn decode_log(&self, new_log: RskLog, contract_info: &ContractInfo)
    -> Result<Option<RskEvent>>;
    fn disconnect(&self) -> Result<()>;
}

#[derive(Error, Debug)]
pub enum RskSubscriptionError {
    #[error("Connection with provider closed")]
    ClosedConnection,
    #[error("Subscription lagged {0} messages behind node")]
    Lagged(u64),
    #[error("Transient error on subscription: {0}")]
    Transient(&'static str), // TODO in the future we could consider discarding related item (ie. log for address) after N errors
    #[error("Unexpected error on subscription: {0}")]
    Unexpected(anyhow::Error),
}
