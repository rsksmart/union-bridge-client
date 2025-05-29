use crate::types::RskPegManagerEvents;
use anyhow::Result;

mod advance_funds;

pub use advance_funds::advance_funds_pegout_processor::*;
use common::types::RskBlockAndUncles;

pub trait EventProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()>;

    fn process_new_block(&mut self, _block: &RskBlockAndUncles) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn shutdown(&mut self);
}
