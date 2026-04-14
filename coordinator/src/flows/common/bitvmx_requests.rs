use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::CommsAddress;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CommInfoRequest {
    pub req_id: Option<Uuid>,
    pub value: Option<CommsAddress>,
}

impl CommInfoRequest {
    pub fn new() -> Self {
        Self { req_id: None, value: None }
    }

    pub fn start(&mut self, req_id: Uuid) {
        self.req_id = Some(req_id);
        self.value = None;
    }

    pub fn complete(&mut self, comm_info: CommsAddress) {
        self.value = Some(comm_info);
    }

    pub fn matches(&self, req_id: &Uuid) -> bool {
        self.req_id.is_some_and(|stored_req_id| stored_req_id == *req_id)
    }

    pub fn get(&self) -> Result<&CommsAddress> {
        self.value.as_ref().context("CommInfo request not completed yet")
    }
}
