use crate::types::{ContractInfo, RskBlock, RskLog};
use anyhow::Error as AnyhowError;
use anyhow::Result;
use serde_json::Value;
use thiserror::Error;

#[cfg(feature = "generate-mocks")]
use mockall::automock;

#[cfg_attr(feature = "generate-mocks", automock)]
pub trait RskSubscription<T> {
    fn next(&mut self) -> Result<T, RskProviderError>;
    fn unsubscribe(&self) -> Result<()>;
}

#[derive(Debug)]
pub struct RskSubscriptionFilter {
    pub addresses: Vec<String>, // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-43
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub topics: Vec<String>, // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-43
}

impl RskSubscriptionFilter {
    pub fn new_logs_by_address(addresses: Vec<String>) -> Self {
        Self {
            addresses,
            from_block: None,
            to_block: None,
            topics: vec![],
        }
    }

    pub fn new_custom(
        addresses: Vec<String>,
        from_block: Option<u64>,
        to_block: Option<u64>,
        topics: Vec<String>,
    ) -> Self {
        Self {
            addresses,
            from_block,
            to_block,
            topics,
        }
    }
}

#[cfg_attr(feature = "generate-mocks", automock(
    type BlockSubscription = MockRskSubscription<RskBlock>;
    type LogSubscription = MockRskSubscription<RskLog>;
))]
pub trait RskProvider {
    type BlockSubscription: RskSubscription<RskBlock>;
    type LogSubscription: RskSubscription<RskLog>;

    fn subscribe_blocks(&self) -> Result<Self::BlockSubscription>;
    fn subscribe_logs(&self, filter: RskSubscriptionFilter) -> Result<Self::LogSubscription>;
    fn get_block_by_hash(&self, hash: &str) -> Result<Option<RskBlock>>;
    fn get_block_by_number(&self, num: u64) -> Result<Option<RskBlock>>;
    fn get_best_block(&self) -> Result<RskBlock>;
    fn decode_log(&self, new_log: &RskLog, contract_info: &ContractInfo) -> Result<Option<Value>>;
    fn disconnect(&self) -> Result<()>;
}

#[derive(Error, Debug)]
pub enum RskProviderError {
    #[error("Connection with provider closed")]
    Closed,
    #[error("Unexpected format from provider: {0}")]
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
