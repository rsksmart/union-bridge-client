use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::types::RskBlock;

mod advance_funds;

pub use advance_funds::advance_funds_pegout_processor::*;

pub trait EventProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()>;

    fn process_new_block(&mut self, _block: &RskBlock) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn shutdown(&mut self);
}
