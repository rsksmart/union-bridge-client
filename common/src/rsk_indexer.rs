use crate::rsk_provider::RskProvider;
use anyhow::Result;

pub trait RskIndexer<P, S>
where
    P: RskProvider,
{
    fn run(&self) -> Result<()>;
}
