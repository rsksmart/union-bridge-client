use anyhow::{Context, Result};
use common_core::types::{Address, BlockHash, BlockNumber, RskBlock, RskLog};
use common_runtime::config::{IndexerConfig, IndexerStartFrom};
use mockall::automock;
use thiserror::Error;
use tracing::info;

#[derive(Debug)]
pub enum BlockNumRef {
    Latest,
    Number(u64),
}

#[automock]
pub trait RskSubscription<T> {
    /// # Errors
    ///
    /// Returns an error if the subscription fails.
    fn next(&mut self) -> Result<T, RskSubscriptionError>;
    /// # Errors
    ///
    /// Returns an error if the unsubscribe operation fails.
    fn unsubscribe(&self) -> Result<()>;
}

#[derive(Debug, PartialEq, Clone)]
pub struct RskSubscriptionFilter {
    pub addresses: Vec<Address>,
    pub topics: Vec<String>,
    pub from_block: Option<BlockNumber>,
}

impl RskSubscriptionFilter {
    #[must_use]
    pub fn new(
        addresses: Vec<Address>,
        topics: Vec<String>,
        from_block: Option<BlockNumber>,
    ) -> Self {
        Self { addresses, topics, from_block }
    }
}

#[automock(
    type BlockSubscription = MockRskSubscription<RskBlock>;
    type LogSubscription = MockRskSubscription<RskLog>;
)]
pub trait RskProvider {
    type BlockSubscription: RskSubscription<RskBlock>;
    type LogSubscription: RskSubscription<RskLog>;

    /// # Errors
    ///
    /// Returns an error if the subscription fails.
    fn subscribe_blocks(&self) -> Result<Self::BlockSubscription>;
    /// # Errors
    ///
    /// Returns an error if the subscription fails.
    fn subscribe_logs(&self, filter: RskSubscriptionFilter) -> Result<Self::LogSubscription>;
    /// # Errors
    ///
    /// Returns an error if the block cannot be retrieved.
    fn get_block_by_hash(&self, hash: BlockHash) -> Result<Option<RskBlock>>;
    /// # Errors
    ///
    /// Returns an error if the block cannot be retrieved.
    fn get_block_by_number(&self, num: BlockNumber) -> Result<Option<RskBlock>>;
    /// # Errors
    ///
    /// Returns an error if the uncle block cannot be retrieved.
    fn get_uncle_by_hash_and_index(&self, hash: BlockHash, index: u64) -> Result<Option<RskBlock>>;
    /// # Errors
    ///
    /// Returns an error if the best block cannot be retrieved.
    fn get_best_block(&self) -> Result<RskBlock>;
    /// # Errors
    ///
    /// Returns an error if the logs cannot be retrieved.
    fn get_logs(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        addrs: &[Address],
    ) -> Result<Vec<RskLog>>;
    /// # Errors
    ///
    /// Returns an error if the disconnect operation fails.
    fn disconnect(&self) -> Result<()>;
}

#[derive(Error, Debug)]
pub enum RskSubscriptionError {
    #[error("Connection with provider closed")]
    ClosedConnection,
    #[error("Subscription lagged {0} messages behind node")]
    Lagged(u64),
    #[error("Transient error on subscription: {0}")]
    Transient(&'static str),
    #[error("Unexpected error on subscription: {0}")]
    Unexpected(anyhow::Error),
}

/// Resolves the initial block based on the indexer's `start_from` configuration.
///
/// # Panics
///
/// Panics when `start_from = "hash"` and `initial_block_hash` is missing or cannot be parsed
/// as a valid block hash.
///
/// # Errors
///
/// Returns an error when:
/// - `start_from = "hash"` and the provider fails to retrieve the block by hash, or the block
///   is not found on the provider.
/// - `start_from = "best"` and the provider fails to retrieve the current best block.
pub fn resolve_initial_block<P: RskProvider>(
    config: &IndexerConfig,
    provider: &P,
) -> Result<RskBlock> {
    let block = match config.start_from {
        IndexerStartFrom::Hash => {
            let hash_from_cfg = config
                .initial_block_hash
                .as_deref()
                .context("Missing indexer.initial_block_hash when indexer.start_from is 'hash'")?;

            let initial_block_hash = BlockHash::try_from(hash_from_cfg)
                .with_context(|| format!("Invalid initial block hash: {hash_from_cfg}"))?;

            let block_by_hash = provider
                .get_block_by_hash(initial_block_hash)
                .context("Failed to get initial block by hash")?
                .context("Initial block not found on provider")?;

            info!(
                "Indexer start_from 'hash': using initial block {} ({})",
                block_by_hash.hash(),
                block_by_hash.number()
            );

            block_by_hash
        }
        IndexerStartFrom::Best => {
            let best_block = provider
                .get_best_block()
                .context("Failed to get best block for start_from='best'")?;

            info!(
                "Indexer start_from 'best': using best block {} ({})",
                best_block.hash(),
                best_block.number()
            );

            best_block
        }
    };

    Ok(block)
}
