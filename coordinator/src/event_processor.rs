use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::{msg_broker::types::BrokerResponses, types::RskBlock};

mod disputed_pegout_processor;
pub use disputed_pegout_processor::*;

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

    fn process_new_block(&mut self, _block: &RskBlock) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn waiting_blocks(&self) -> bool {
        // default false
        false
    }

    fn shutdown(&self);
}
