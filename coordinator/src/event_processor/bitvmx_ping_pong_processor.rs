use crate::event_processor::EventProcessor;
use alloy_primitives::BlockNumber;
use anyhow::{Result, bail};
use bitvmx_client::types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::{BROKER_SERVER_ID, BrokerClientApi};
use common::msg_broker::types::{FromServer, ToServer};
use common::types::RskBlockAndUncles;
use log::{debug, info, trace};

pub struct BitVmxPingPongProcessor<BC: BrokerClientApi> {
    bitvmx_broker: BC,
    ping_block: Option<BlockNumber>,
}

impl<BC: BrokerClientApi> BitVmxPingPongProcessor<BC> {
    pub fn new(bitvmx_broker: BC) -> Self {
        Self {
            bitvmx_broker,
            ping_block: None,
        }
    }
}

const ROUND_BLOCK_INTERVAL: u64 = 2;

impl<BC: BrokerClientApi> EventProcessor for BitVmxPingPongProcessor<BC> {
    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        let block_num = block.number().value();

        if let Some(ping_block) = self.ping_block {
            if block_num > ping_block + ROUND_BLOCK_INTERVAL {
                bail!("BitVMX Pong not received after {ROUND_BLOCK_INTERVAL} blocks")
            }
        }

        if block_num % ROUND_BLOCK_INTERVAL == 0 {
            info!("Sending Ping to BitVMX at block {block_num}");
            self.bitvmx_broker.send(
                BROKER_SERVER_ID,
                ToServer::ToBitVMX(IncomingBitVMXApiMessages::Ping()),
            )?;

            self.ping_block = Some(block_num);
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> Result<()> {
        match event {
            FromServer::FromBitVMX(m) => match m {
                OutgoingBitVMXApiMessages::Pong() => {
                    debug!("Received Pong from BitVMX, closing round {:?}", m);
                    self.ping_block = None;
                }
                _ => {
                    debug!("Discarded non Pong message on PingPongProcessor: {:?}", m);
                }
            },
            _ => {
                trace!(
                    "Discarded Union Client event on PingPongProcessor: {:?}",
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
