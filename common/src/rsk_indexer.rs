use anyhow::Result;

use crate::rsk_provider::RskProvider;

pub trait RskIndexer<P, S>
where
    P: RskProvider,
{
    fn run(&self) -> Result<()>;
}
