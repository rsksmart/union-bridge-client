use crate::types::RskPegManagerEvents;
use anyhow::Result;
use common::{msg_broker::types::FromServer, types::RskBlockAndUncles};
use log::info;

mod advance_funds;
pub use advance_funds::advance_funds_pegout_processor::*;

mod get_temporary_pegin_address_processor;
pub use get_temporary_pegin_address_processor::*;

#[derive(Debug)]
pub struct Confirmations {
    flow_id: String,
    accum: u32,
    req: u32,
}

impl Confirmations {
    pub fn new(flow_id: String, req_confirmations: u32) -> Self {
        Self {
            flow_id,
            accum: 0,
            req: req_confirmations,
        }
    }

    pub fn update(&mut self, removed: bool) {
        if removed {
            self.accum = self.accum.saturating_sub(1);
            info!(
                "Removed confirmation for {}. Status: {}/{}",
                self.flow_id, self.accum, self.req
            );
        } else {
            self.accum = self.accum.saturating_add(1);
            info!(
                "Added confirmation to {}. Status: {}/{}",
                self.flow_id, self.accum, self.req
            );
        }
    }

    pub fn is_confirmed(&self) -> bool {
        self.accum >= self.req
    }
}

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
