use anyhow::{Context, Result, bail};
use common::msg_broker::broker::BrokerServer;
use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use common::types::RskLog;
use log::{debug, error, info, trace, warn};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;

pub struct Notifier {
    new_log_channel: mpsc::Receiver<RskLog>,
    msg_broker: BrokerServer,
    topics_with_consumers: HashMap<String, HashSet<u32>>,
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
            topics_with_consumers: HashMap::new(),
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
                // try to receive new ASAP
                self.notify_consumers(log)?;
            } else {
                trace!("No new logs yet, sleep a bit");
                thread::sleep(std::time::Duration::from_secs(5)); // TODO(iago) config   
            }
        }

        info!("Shutdown requested, stopping notifier");

        Ok(())
    }
    fn update_consumers(&mut self) -> Result<()> {
        match self.msg_broker.try_recv()? {
            Some((BrokerRequests::SubscribeLogs(topic), consumer_id)) => {
                info!("New consumer {consumer_id} subscribing to topic {topic}");
                self.topics_with_consumers
                    .entry(topic)
                    .or_insert_with(HashSet::new)
                    .insert(consumer_id);
            }
            Some((BrokerRequests::UnsubscribeLogs(topic), consumer_id)) => {
                self.unsubscribe_consumer_from_topic(topic, &consumer_id);
            }
            Some((_, consumer_id)) => {
                warn!(
                    "Unexpected request type on Notifier from consumer {consumer_id}, unsubscribing"
                );
                self.unsubscribe_consumer_from_all_topics(&consumer_id);
            }
            None => {
                trace!("No messages in Notifier's msg_broker");
            }
        }

        Ok(())
    }

    fn unsubscribe_consumer_from_topic(&mut self, topic: String, consumer_id: &u32) {
        info!("Unsubscribing consumer {consumer_id}");
        if let Entry::Occupied(mut consumer) = self.topics_with_consumers.entry(topic) {
            consumer.get_mut().remove(&consumer_id);
            let consumer_topics = consumer.get();
            if consumer_topics.is_empty() {
                consumer.remove_entry();
            }
        }
    }

    fn unsubscribe_consumer_from_all_topics(&mut self, consumer_id: &u32) {
        info!("Unsubscribing consumer {consumer_id} from all topics");
        self.topics_with_consumers.retain(|_, consumers| {
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
        let response = BrokerResponses::Log(new_log);

        let topic = "test_topic"; // TODO(iago): Replace with actual topic from new_log
        let consumers_for_topic = self.topics_with_consumers.get(topic);
        if let Some(consumers_for_topic) = consumers_for_topic {
            for c_id in consumers_for_topic {
                debug!("Notifying consumer {c_id} about new log {topic}");

                self.msg_broker
                    .send(&response, *c_id)
                    .context(format!("Sending log {topic} to consumer {c_id}"))?;
            }
        } else {
            debug!("No consumers for topic {topic}");
        }

        Ok(())
    }
}
