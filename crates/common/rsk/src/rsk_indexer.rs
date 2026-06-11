use anyhow::Result;

use crate::rsk_provider::RskProvider;

pub trait RskIndexer<P, S>
where
    P: RskProvider,
{
    /// # Errors
    ///
    /// Returns an error if the indexer fails to run.
    fn run(&self) -> Result<()>;
}
