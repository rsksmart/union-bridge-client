use crate::types::{RskBlock, RskLog};
use anyhow::Result;

pub trait RskSubscription<T> {
    fn next(&mut self) -> Result<Option<T>>;
    fn unsubscribe(&self) -> Result<()>;
}

pub trait RskProvider {
    fn subscribe_blocks(&self) -> Result<impl RskSubscription<RskBlock>>;
    fn subscribe_logs(&self) -> Result<impl RskSubscription<RskLog>>;
    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>>;
    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn disconnect(&self) -> Result<()>;
}
