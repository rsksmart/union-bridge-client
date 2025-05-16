use anyhow::{Context, Result};
use common::constants::coordinator::MONITOR_CHECK_PERIOD;
use common::msg_broker::broker::BrokerServerApi;
use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use common::types::RskBlock;
use log::{debug, info, trace, warn};
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::Duration;

pub struct Notifier<BS: BrokerServerApi> {
    new_block_channel: mpsc::Receiver<RskBlock>,
    msg_broker: BS,
    check_period: Duration,
    consumers: HashSet<u32>,
    shutdown_flag: ShutdownFlag,
}

impl<BS: BrokerServerApi> Notifier<BS> {
    pub fn new(
        indexer_receiver: mpsc::Receiver<RskBlock>,
        msg_broker: BS,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_block_channel: indexer_receiver,
            msg_broker,
            check_period: MONITOR_CHECK_PERIOD,
            consumers: HashSet::new(),
            shutdown_flag,
        }
    }

    #[cfg(test)]
    pub fn new_for_tests(
        indexer_receiver: mpsc::Receiver<RskBlock>,
        msg_broker: BS,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_block_channel: indexer_receiver,
            msg_broker,
            check_period: Duration::from_millis(1),
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
                thread::sleep(self.check_period);
            }
        }

        info!("Shutdown requested, stopping notifier");

        Ok(())
    }
    fn update_consumers(&mut self) -> Result<()> {
        match self.msg_broker.try_recv()? {
            Some((BrokerRequests::SubscribeBlocks, consumer_id)) => {
                info!("New consumer {consumer_id} for blocks");
                self.consumers.insert(consumer_id);
            }
            Some((BrokerRequests::UnsubscribeBlocks, consumer_id)) => {
                info!("Unsubscribing consumer {consumer_id}");
                let removed = self.consumers.remove(&consumer_id);
                if !removed {
                    trace!("Consumer {consumer_id} was not subscribed to blocks");
                }
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
            Err(TryRecvError::Disconnected) => match self.shutdown_flag.is_on() {
                true => Ok(None),
                false => Err(anyhow::anyhow!("Indexer channel disconnected")),
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use common::msg_broker::broker::MockBrokerServerApi;
    use common::test_utils::rsk_block_generator::{
        get_first_default_rsk_block, get_second_default_rsk_block,
    };
    use std::sync::mpsc;
    use std::sync::mpsc::Sender;
    use std::thread::{JoinHandle, sleep};
    use std::time::Duration;

    #[test]
    fn test_run_new_block_received_no_consumers() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let expected_block = get_first_default_rsk_block();

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker.expect_try_recv().returning(|| Ok(None)); // no subscription message
        mock_broker.expect_send().never(); // nothing to send, no consumers yet

        let mut notifier = Notifier::new_for_tests(rx, mock_broker, shutdown_flag.clone());

        let handle_external_events =
            handle_external_events(tx, shutdown_flag, vec![expected_block]);

        let result = notifier.run();

        handle_external_events
            .join()
            .expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
            panic!("Run failed: {:?}", e);
        }
    }

    #[test]
    fn test_run_new_block_received_no_events() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker
            .expect_try_recv()
            .returning(|| Ok(Some((BrokerRequests::SubscribeBlocks, 1)))); // subscribe for a different address
        mock_broker.expect_send().never(); // nothing to send, no consumers yet for that address

        let mut notifier = Notifier::new_for_tests(rx, mock_broker, shutdown_flag.clone());

        let handle_external_events = handle_external_events(tx, shutdown_flag, vec![]);

        let result = notifier.run();

        handle_external_events
            .join()
            .expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
            panic!("Run failed: {:?}", e);
        }
    }

    #[test]
    fn test_run_new_block_received_with_consumer() {
        let client_id = 2;

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests = vec![ClientRequest {
            id: client_id,
            request: BrokerRequests::SubscribeBlocks,
        }];

        let expected_block_1 = get_first_default_rsk_block();
        let expected_block_2 = get_second_default_rsk_block();

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv_subscribe(client_requests, &mut mock_broker_server);

        expect_send_block(client_id, &expected_block_1, &mut mock_broker_server);
        expect_send_block(client_id, &expected_block_2, &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events =
            handle_external_events(tx, shutdown_flag, vec![expected_block_1, expected_block_2]);

        let result = notifier.run();

        handle_external_events
            .join()
            .expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
            panic!("Run failed: {:?}", e);
        }
    }

    #[test]
    fn test_run_new_block_received_with_multiple_consumers() {
        let client_id_1 = 2;
        let client_id_2 = 3;

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests = vec![
            ClientRequest {
                id: client_id_1,
                request: BrokerRequests::SubscribeBlocks,
            },
            ClientRequest {
                id: client_id_2,
                request: BrokerRequests::SubscribeBlocks,
            },
        ];

        let expected_block_1 = get_first_default_rsk_block();
        let expected_block_2 = get_second_default_rsk_block();

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv_subscribe(client_requests, &mut mock_broker_server);

        expect_send_block(client_id_1, &expected_block_1, &mut mock_broker_server);
        expect_send_block(client_id_2, &expected_block_1, &mut mock_broker_server);
        expect_send_block(client_id_1, &expected_block_2, &mut mock_broker_server);
        expect_send_block(client_id_2, &expected_block_2, &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events =
            handle_external_events(tx, shutdown_flag, vec![expected_block_1, expected_block_2]);

        let result = notifier.run();

        handle_external_events
            .join()
            .expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
            panic!("Run failed: {:?}", e);
        }
    }

    #[test]
    fn test_run_unsubscribe() {
        let client_id_1 = 2;
        let client_id_2 = 3;

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests = vec![
            ClientRequest {
                id: client_id_1,
                request: BrokerRequests::SubscribeBlocks,
            },
            ClientRequest {
                id: client_id_2,
                request: BrokerRequests::SubscribeBlocks,
            },
            // should not receive blocks for this address
            ClientRequest {
                id: client_id_1,
                request: BrokerRequests::UnsubscribeBlocks,
            },
        ];

        let expected_block_1 = get_first_default_rsk_block();
        let expected_block_2 = get_second_default_rsk_block();

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv_subscribe(client_requests, &mut mock_broker_server);

        expect_send_block(client_id_2, &expected_block_1, &mut mock_broker_server);
        expect_send_block(client_id_2, &expected_block_2, &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events =
            handle_external_events(tx, shutdown_flag, vec![expected_block_1, expected_block_2]);

        let result = notifier.run();

        handle_external_events
            .join()
            .expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
            panic!("Run failed: {:?}", e);
        }
    }

    fn expect_try_recv_subscribe(
        client_requests: Vec<ClientRequest>,
        mock_broker_server: &mut MockBrokerServerApi,
    ) {
        use std::collections::VecDeque;

        mock_broker_server.expect_try_recv().returning_st({
            let mut responses: VecDeque<_> = client_requests
                .into_iter()
                .map(|coa| Ok(Some((coa.request, coa.id))))
                .collect();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn expect_send_block(
        dest: u32,
        expected_block: &RskBlock,
        mock_broker_server: &mut MockBrokerServerApi,
    ) {
        mock_broker_server
            .expect_send()
            .withf({
                let expected_block = expected_block.clone(); // move into closure
                move |response, consumer_id| match response {
                    BrokerResponses::Block(actual_block) => {
                        *consumer_id == dest && *actual_block == expected_block
                    }
                    _ => false,
                }
            })
            .returning(|_, _| Ok(()))
            .once();
    }

    fn handle_external_events(
        tx: Sender<RskBlock>,
        shutdown_flag: ShutdownFlag,
        blocks: Vec<RskBlock>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            // give time for subscriptions to be processed
            sleep(Duration::from_millis(10));

            for block in blocks {
                tx.send(block).expect("Failed to send block");
            }

            // give time for messages to be processed
            sleep(Duration::from_millis(10));

            shutdown_flag.set();
        })
    }

    struct ClientRequest {
        id: u32,
        request: BrokerRequests,
    }
}
