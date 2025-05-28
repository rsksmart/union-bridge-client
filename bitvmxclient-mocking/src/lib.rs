use anyhow::{Context, Result};
use common::{
    contracts::types::PegInAddressInput,
    msg_broker::{
        broker::BrokerServerApi,
        types::{BrokerRequests, BrokerResponses},
    },
};
use std::collections::HashSet;

pub struct Executor<BS: BrokerServerApi> {
    broker_server: BS,
    consumers: HashSet<u32>,
}

impl<BS: BrokerServerApi> Executor<BS> {
    pub fn new(broker_server: BS) -> Self {
        Self {
            broker_server,
            consumers: HashSet::new(),
        }
    }

    pub fn update_consumers(&mut self) -> Result<()> {
        match self.broker_server.try_recv()? {
            Some((BrokerRequests::SubscribeBitVMX, consumer_id)) => {
                println!("Status: New consumer {consumer_id} for BitVMX messages");
                self.consumers.insert(consumer_id);
            }
            Some((BrokerRequests::UnsubscribeBitVMX, consumer_id)) => {
                println!("Status: Unsubscribing consumer {consumer_id}");
                let removed = self.consumers.remove(&consumer_id);
                if !removed {
                    println!("Status: Consumer {consumer_id} was not subscribed to BitVMX messages");
                }
            }
            Some((_, consumer_id)) => {
                println!("Status: Unexpected request type from consumer {consumer_id}, unsubscribing");
                self.consumers.remove(&consumer_id);
            }
            None => {
                println!("Status: No messages in broker");
            }
        }

        Ok(())
    }

    pub fn send_get_temporary_pegin_address_event(
        &self,
        rootstock_deposit_address: String,
        value: u64,
        btc_reimbursement_pub_key: String,
    ) -> Result<()> {
        let payload = PegInAddressInput {
            rootstock_deposit_address,
            value,
            btc_reimbursement_pub_key,
        };

        let event = BrokerResponses::GetTemporaryPegInAddress(payload);

        self.notify_consumers(event)
    }

    fn notify_consumers(&self, event: BrokerResponses) -> Result<()> {
        for c_id in &self.consumers {
            println!("Status: Notifying consumer {} about new event {:?}", c_id, event);

            self.broker_server
                .send(&event, *c_id)
                .context(format!("sending event {:?} to consumer {}", event, c_id))?;
        }

        Ok(())
    }
}
