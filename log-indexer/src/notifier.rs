use anyhow::{Context, Result};
use common::constants::coordinator::MONITOR_CHECK_PERIOD;
use common::msg_broker::broker::BrokerServer;
use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use common::types::{Address, RskLog};
use log::{debug, info, trace, warn};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;

pub struct Notifier {
    new_log_channel: mpsc::Receiver<RskLog>,
    msg_broker: BrokerServer,
    contracts_with_consumers: HashMap<Address, HashSet<u32>>,
    shutdown_flag: ShutdownFlag,
}

impl Notifier {
    pub fn new(
        indexer_receiver: mpsc::Receiver<RskLog>,
        msg_broker: BrokerServer,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_log_channel: indexer_receiver,
            msg_broker,
            contracts_with_consumers: HashMap::new(),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.shutdown_flag.is_on() {
                break;
            }

            self.update_consumers()?;

            if let Some(log) = self.try_new_log()? {
                self.notify_consumers(log)?;
                // no sleep, try to receive new ASAP
            } else {
                trace!("No new logs yet, sleep a bit");
                thread::sleep(MONITOR_CHECK_PERIOD);
            }
        }

        info!("Shutdown requested, stopping notifier");

        Ok(())
    }
    fn update_consumers(&mut self) -> Result<()> {
        match self.msg_broker.try_recv()? {
            Some((BrokerRequests::SubscribeLogs(event), consumer_id)) => {
                self.subscribe_consumer_to_contract(event, consumer_id);
            }
            Some((BrokerRequests::UnsubscribeLogs(topic), consumer_id)) => {
                self.unsubscribe_consumer_from_contract(topic, consumer_id);
            }
            Some((_, consumer_id)) => {
                warn!(
                    "Unexpected request type on Notifier from consumer {consumer_id}, unsubscribing"
                );
                self.unsubscribe_consumer_from_all_contracts(&consumer_id);
            }
            None => {
                trace!("No messages in Notifier's msg_broker");
            }
        }

        Ok(())
    }

    fn subscribe_consumer_to_contract(&mut self, address: Address, consumer_id: u32) {
        info!("New consumer {consumer_id} subscribing to {address}");
        self.contracts_with_consumers
            .entry(address)
            .or_insert_with(HashSet::new)
            .insert(consumer_id);
    }

    fn unsubscribe_consumer_from_contract(&mut self, address: Address, consumer_id: u32) {
        info!("Unsubscribing consumer {consumer_id}");
        if let Entry::Occupied(mut consumer) = self.contracts_with_consumers.entry(address) {
            consumer.get_mut().remove(&consumer_id);
            let consumer_contracts = consumer.get();
            if consumer_contracts.is_empty() {
                consumer.remove_entry();
            }
        } else {
            debug!("Consumer {consumer_id} was not subscribed to {address}");
        }
    }

    fn unsubscribe_consumer_from_all_contracts(&mut self, consumer_id: &u32) {
        info!("Unsubscribing consumer {consumer_id} from all contracts");
        self.contracts_with_consumers.retain(|_, consumers| {
            consumers.remove(consumer_id);
            !consumers.is_empty()
        });
    }

    fn try_new_log(&mut self) -> Result<Option<RskLog>> {
        match self.new_log_channel.try_recv() {
            Ok(b) => {
                debug!("New log received by notifier {:?}", b);
                Ok(Some(b))
            }
            Err(TryRecvError::Empty) => {
                trace!("No new log yet");
                Ok(None)
            }
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("Indexer channel disconnected")),
        }
    }

    fn notify_consumers(&mut self, new_log: RskLog) -> Result<()> {
        let selector = new_log.selector();
        let address: Address = new_log.info().address();
        let response = BrokerResponses::Log(new_log);

        if let Some(consumers_for_contract) = self.contracts_with_consumers.get(&address) {
            for c_id in consumers_for_contract {
                debug!("Notifying {selector} to consumer {c_id}");

                self.msg_broker
                    .send(&response, *c_id)
                    .context(format!("Sending {selector} to consumer {c_id}"))?;
            }
        } else {
            debug!("No consumers for event {selector}");
        }

        Ok(())
    }
}
