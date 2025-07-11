use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::BitVmxBrokerServerApi;

pub struct Executor<BS: BitVmxBrokerServerApi> {
    broker_server: BS,
}

impl<BS: BitVmxBrokerServerApi> Executor<BS> {
    pub fn new(broker_server: BS) -> Self {
        Self { broker_server }
    }

    pub fn try_recv(&mut self) -> Result<()> {
        match self.broker_server.try_recv()? {
            Some((IncomingBitVMXApiMessages::GenerateZKP(id, data), from)) => {
                println!(
                    "Received GenerateZKP from {from} with id {id} and data {:?} at {}",
                    hex::encode(data),
                    Self::reception_time()
                );
            }
            Some((IncomingBitVMXApiMessages::Ping(), from)) => {
                // println!("Received Ping from {from} at {}", Self::reception_time());

                self.broker_server
                    .send(&OutgoingBitVMXApiMessages::Pong(), from)
                    .context("Failed to send Pong response")?;
            }
            Some((msg, _from)) => {
                println!(
                    "Unexpected IncomingBitVMXApiMessages received {:?} at {}",
                    msg,
                    Self::reception_time()
                );
            }
            None => {
                // No message received
            }
        }

        Ok(())
    }

    fn reception_time() -> String {
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
    }
}
