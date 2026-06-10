use anyhow::Result;
use chrono::{DateTime, Utc};
use common_bitvmx::bitvmx_types::OutgoingBitVMXApiMessages;
use common_core::types::RskBlockAndUncles;
#[cfg(test)]
use mockall::automock;
use serde::Serialize;

use crate::types::{FlowKind, RskPegManagerEvents, UserRequests};

#[derive(Serialize)]
pub struct FlowDetails {
    pub kind: FlowKind,
    pub id: String,
    pub step: String,
    /// `None` for flows persisted before the `created_at` field existed.
    pub created_at: Option<DateTime<Utc>>,
}

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

    fn active_flows(&self) -> Vec<FlowDetails> {
        // default no flows reported
        Vec::new()
    }
}
