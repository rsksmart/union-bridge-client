use crate::event_processor::EventProcessor;
use crate::types::UserRequests;
use anyhow::Result;
use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use log::{info, trace};
use std::rc::Rc;
use uuid::Uuid;

pub(crate) struct FundBitvmxProcessor<BC>
where
    BC: BitVmxBrokerClientApi,
{
    bitvmx_broker: Rc<BC>,
}

impl<BC> FundBitvmxProcessor<BC>
where
    BC: BitVmxBrokerClientApi,
{
    pub fn new(bitvmx_broker: Rc<BC>) -> Self {
        Self { bitvmx_broker }
    }
}

impl<BC> EventProcessor for FundBitvmxProcessor<BC>
where
    BC: BitVmxBrokerClientApi,
{
    fn process_user_request(&mut self, event: &UserRequests) -> Result<()> {
        match event {
            UserRequests::GetBitVMXFundingAddress => {
                let id = Uuid::new_v4();
                self.bitvmx_broker.send(
                    BROKER_SERVER_ID,
                    IncomingBitVMXApiMessages::GetFundingAddress(id),
                )?;
            }
            _ => {
                trace!("FundBitvmxProcessor: Ignoring user request {:?}", event);
            }
        }
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::FundingAddress(_req_id, addr) => {
                info!("=== Received BitVMX Funding Address: {addr:?} ===");
                // a webhook to the user app would be sent here
            }
            _ => {
                trace!("FundBitvmxProcessor: Ignoring BitVMX event {:?}", event);
            }
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        // nothing special to do here
    }
}
