use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::msg_broker::bitvmx_types::OutgoingBitVMXApiMessages;
use common::types::RskBlockAndUncles;

mod pegin_processor;
mod pegout_processor;

pub use pegin_processor::*;
pub use pegout_processor::*;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait EventProcessor {
    fn process_new_bitvmx_event(&mut self, _event: &OutgoingBitVMXApiMessages) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn process_new_rsk_event(&mut self, _event: &RskPegManagerEvents) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn process_new_block(&mut self, _block: &RskBlockAndUncles) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn shutdown(&mut self);
}
