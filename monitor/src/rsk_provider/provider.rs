use crate::types::RskBlock;
use anyhow::Result;

pub trait RskBlockSubscription {
    fn next(&mut self) -> Result<Option<RskBlock>>;
    fn unsubscribe(&self) -> Result<()>;
}

pub trait RskProvider {
    fn subscribe_blocks(&self) -> Result<impl RskBlockSubscription>;
    fn get_block_by_hash(&self, hash: &str) -> Result<RskBlock>;
    fn get_block_by_number(&self, num: u64) -> Result<RskBlock>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn disconnect(&self) -> Result<()>;
}
