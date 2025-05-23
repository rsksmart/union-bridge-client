use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::types::RskBlock;

mod disputed_pegout_processor;

pub use disputed_pegout_processor::*;

pub trait EventProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()>;

    fn process_new_block(&mut self, _block: &RskBlock) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn is_waiting_blocks(&self) -> bool {
        // default false
        false
    }

    fn shutdown(&mut self);
}
