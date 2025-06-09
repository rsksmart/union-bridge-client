use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::{msg_broker::types::BrokerResponses, types::RskBlockAndUncles};

mod advance_funds;
pub use advance_funds::advance_funds_pegout_processor::*;

mod get_temporary_pegin_address_processor;
pub use get_temporary_pegin_address_processor::*;

pub trait EventProcessor {
    fn process_new_bitvmx_event(&mut self, _event: &BrokerResponses) -> Result<()> {
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
