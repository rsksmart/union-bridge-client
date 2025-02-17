use crate::shutdown_flag::ShutdownFlag;
use crate::types::{RskBlock, RskLog};
use anyhow::Error as AnyhowError;
use anyhow::Result;
use thiserror::Error;

use mockall::*;
use mockall::predicate::*;
#[automock]
pub trait RskSubscription<T> {
    fn next(&mut self) -> Result<T, RskProviderError>;
    fn unsubscribe(&self) -> Result<()>;
}

#[automock(
    type BlockSubscription=MockRskSubscription<RskBlock>;
    type LogSubscription=MockRskSubscription<RskLog>;
)]
pub trait RskProvider {
    type BlockSubscription: RskSubscription<RskBlock>;
    type LogSubscription: RskSubscription<RskLog>;
    fn subscribe_blocks(
        &self,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self::BlockSubscription>;
    fn subscribe_logs(&self, shutdown_flag: ShutdownFlag) -> Result<Self::LogSubscription>;
    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>>;
    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn disconnect(&self) -> Result<()>;
}

#[derive(Error, Debug)]
pub enum RskProviderError {
    #[error("Connection with provider closed")]
    Closed,
    #[error("Unexpected response from provider: {0}")]
    Format(#[from] serde_json::Error),
    #[error("Unknown error: {0}")]
    Other(String),
}

// TODO(Jira) think if this should be removed in scope of https://rsklabs.atlassian.net/browse/UB-28
impl From<AnyhowError> for RskProviderError {
    fn from(error: AnyhowError) -> Self {
        let message = format!("{:?}", error);
        RskProviderError::Other(message)
    }
}