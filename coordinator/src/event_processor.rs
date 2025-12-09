use anyhow::Result;
use common::msg_broker::bitvmx_types::OutgoingBitVMXApiMessages;
use common::types::RskBlockAndUncles;
#[cfg(test)]
use mockall::automock;

use crate::types::{RskPegManagerEvents, UserRequests};

#[cfg_attr(test, automock)]
pub trait EventProcessor {
    #[allow(clippy::used_underscore_binding)]
    fn process_user_request(&mut self, _event: &UserRequests) -> Result<()> {
        // default no-op
        Ok(())
    }

    #[allow(clippy::used_underscore_binding)]
    fn process_new_bitvmx_event(&mut self, _event: &OutgoingBitVMXApiMessages) -> Result<()> {
        // default no-op
        Ok(())
    }

    #[allow(clippy::used_underscore_binding)]
    fn process_new_rsk_event(&mut self, _event: &RskPegManagerEvents) -> Result<()> {
        // default no-op
        Ok(())
    }

    #[allow(clippy::used_underscore_binding)]
    fn process_new_block(&mut self, _block: &RskBlockAndUncles) -> Result<()> {
        // default no-op
        Ok(())
    }

    fn shutdown(&mut self);
}
