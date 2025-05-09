use anyhow::{Context, Result};
use common::constants::coordinator::MONITOR_CHECK_PERIOD;
use common::msg_broker::broker::BrokerServer;
use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use common::types::RskBlock;
use log::{debug, info, trace, warn};
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;

pub struct Notifier {
    new_block_channel: mpsc::Receiver<RskBlock>,
    msg_broker: BrokerServer,
    consumers: HashSet<u32>,
    shutdown_flag: ShutdownFlag,
}

impl Notifier {
    pub fn new(
        indexer_receiver: mpsc::Receiver<RskBlock>,
        msg_broker: BrokerServer,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_block_channel: indexer_receiver,
            msg_broker,
            consumers: HashSet::new(),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.shutdown_flag.is_on() {
                break;
            }

            self.update_consumers()?;

            if let Some(block) = self.try_new_block()? {
                self.notify_consumers(block)?;
                // no sleep, try to receive new ASAP
            } else {
                trace!("No new blocks yet, sleep a bit");
                thread::sleep(MONITOR_CHECK_PERIOD);
            }
        }

        info!("Shutdown requested, stopping notifier");

        Ok(())
    }
    fn update_consumers(&mut self) -> Result<()> {
        match self.msg_broker.try_recv()? {
            Some((BrokerRequests::SubscribeBlocks, consumer_id)) => {
                info!("New consumer {consumer_id}");
                self.consumers.insert(consumer_id);
            }
            Some((BrokerRequests::UnsubscribeBlocks, consumer_id)) => {
                info!("Unsubscribing consumer {consumer_id}");
                self.consumers.remove(&consumer_id);
            }
            Some((_, consumer_id)) => {
                warn!(
                    "Unexpected request type on Notifier from consumer {consumer_id}, unsubscribing"
                );
                self.consumers.remove(&consumer_id);
            }
            None => {
                trace!("No messages in Notifier's msg_broker");
            }
        }

        Ok(())
    }

    fn try_new_block(&mut self) -> Result<Option<RskBlock>> {
        match self.new_block_channel.try_recv() {
            Ok(b) => {
                debug!("New block received by notifier {:?}", b);
                Ok(Some(b))
            }
            Err(TryRecvError::Empty) => {
                trace!("No new block yet");
                Ok(None)
            }
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("Indexer channel disconnected")),
        }
    }

    fn notify_consumers(&mut self, new_block: RskBlock) -> Result<()> {
        let hash = new_block.hash();
        let number = new_block.number();
        let response = BrokerResponses::Block(new_block);

        for c_id in &self.consumers {
            debug!("Notifying consumer {c_id} about new block {number} ({hash})");

            self.msg_broker.send(&response, *c_id).context(format!(
                "Sending block {} ({}) to consumer {}",
                number, hash, c_id
            ))?;
        }
        Ok(())
    }
}
