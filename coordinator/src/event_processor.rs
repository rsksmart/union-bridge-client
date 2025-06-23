use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::{msg_broker::types::FromServer, types::RskBlockAndUncles};

mod advance_funds;
mod blockchain_tracker;
mod get_temporary_pegin_address_processor;

pub use advance_funds::advance_funds_processor::*;
pub use get_temporary_pegin_address_processor::*;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait EventProcessor {
    fn process_new_bitvmx_event(&mut self, _event: &FromServer) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn process_new_event(&mut self, _event: &RskPegManagerEvents) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn process_new_block(&mut self, _block: &RskBlockAndUncles) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn shutdown(&mut self);
}
