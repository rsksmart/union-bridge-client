use crate::event_processor::EventProcessor;
use alloy_primitives::BlockNumber;
use anyhow::{Result, bail};
use bitvmx_client::types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::{BROKER_SERVER_ID, BrokerClientApi};
use common::types::RskBlockAndUncles;
use log::{debug, info, trace};

pub struct BitVmxPingPongProcessor<BC>
where
    BC: BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
{
    bitvmx_broker: BC,
    ping_block: Option<BlockNumber>,
}

impl<BC> BitVmxPingPongProcessor<BC>
where
    BC: BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
{
    pub fn new(bitvmx_broker: BC) -> Self {
        Self {
            bitvmx_broker,
            ping_block: None,
        }
    }
}

const ROUND_BLOCK_INTERVAL: u64 = 2;

impl<BC> EventProcessor for BitVmxPingPongProcessor<BC>
where
    BC: BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
{
    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        let block_num = block.number().value();

        if let Some(ping_block) = self.ping_block {
            if block_num > ping_block + ROUND_BLOCK_INTERVAL {
                bail!("BitVMX Pong not received after {ROUND_BLOCK_INTERVAL} blocks")
            }
        }

        if block_num % ROUND_BLOCK_INTERVAL == 0 {
            info!("Sending Ping to BitVMX at block {block_num}");
            self.bitvmx_broker
                .send(BROKER_SERVER_ID, IncomingBitVMXApiMessages::Ping())?;

            self.ping_block = Some(block_num);
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::Pong() => {
                debug!("Received Pong from BitVMX, closing round");
                self.ping_block = None;
            }
            _ => {
                trace!(
                    "Discarded BitVMX Client event on PingPongProcessor: {:?}",
                    event
                );
            }
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        // no-op for now
    }
}
