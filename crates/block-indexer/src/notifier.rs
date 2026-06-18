use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use common_broker::broker::{Identifier, UnionBrokerServerApi};
pub use common_broker::types::{FromServer, ToServer};
use common_core::constants::indexer::NOTIFIER_CHECK_PERIOD;
use common_core::types::RskBlockAndUncles;
use common_runtime::shutdown_flag::ShutdownFlag;
use tracing::{info, instrument, trace, warn};

pub struct BlockNotification {
    block: RskBlockAndUncles,
    delivery_ack: mpsc::Sender<Result<()>>,
}

impl BlockNotification {
    #[must_use]
    pub(crate) fn new(block: RskBlockAndUncles, delivery_ack: mpsc::Sender<Result<()>>) -> Self {
        Self { block, delivery_ack }
    }

    #[must_use]
    pub(crate) fn block(&self) -> &RskBlockAndUncles {
        &self.block
    }

    fn into_parts(self) -> (RskBlockAndUncles, mpsc::Sender<Result<()>>) {
        (self.block, self.delivery_ack)
    }

    #[cfg(test)]
    pub(crate) fn acknowledge(self, result: Result<()>) {
        let _ = self.delivery_ack.send(result);
    }
}

pub struct Notifier<BS: UnionBrokerServerApi> {
    new_block_channel: mpsc::Receiver<BlockNotification>,
    msg_broker: BS,
    check_period: Duration,
    consumers: HashSet<Identifier>,
    shutdown_flag: ShutdownFlag,
}

impl<BS: UnionBrokerServerApi> Notifier<BS> {
    pub fn new(
        indexer_receiver: mpsc::Receiver<BlockNotification>,
        msg_broker: BS,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_block_channel: indexer_receiver,
            msg_broker,
            check_period: NOTIFIER_CHECK_PERIOD,
            consumers: HashSet::new(),
            shutdown_flag,
        }
    }

    pub fn new_with_consumer(
        indexer_receiver: mpsc::Receiver<BlockNotification>,
        msg_broker: BS,
        shutdown_flag: ShutdownFlag,
        consumer: Identifier,
    ) -> Self {
        let mut notifier = Self::new(indexer_receiver, msg_broker, shutdown_flag);
        notifier.consumers.insert(consumer);
        notifier
    }

    #[cfg(test)]
    pub fn new_for_tests(
        indexer_receiver: mpsc::Receiver<BlockNotification>,
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

    /// Run the notifier loop
    ///
    /// # Errors
    ///
    /// Returns an error if there's a failure in the message broker or channel communication
    #[instrument(skip_all)]
    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.shutdown_flag.is_on() {
                break;
            }

            self.update_consumers()?;

            if let Some(block) = self.wait_for_block(self.check_period)? {
                self.notify_consumers(block)?;
            }
        }

        info!("Shutdown requested, stopping notifier");

        Ok(())
    }
    fn update_consumers(&mut self) -> Result<()> {
        match self.msg_broker.try_recv()? {
            Some((ToServer::SubscribeBlocks, consumer_id)) => {
                info!("New consumer {consumer_id} for blocks");
                self.consumers.insert(consumer_id);
            }
            Some((ToServer::UnsubscribeBlocks, consumer_id)) => {
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

    fn wait_for_block(&mut self, timeout: Duration) -> Result<Option<BlockNotification>> {
        match self.new_block_channel.recv_timeout(timeout) {
            Ok(notification) => {
                trace!("New block received by notifier {:?}", notification.block());
                Ok(Some(notification))
            }
            Err(RecvTimeoutError::Timeout) => {
                trace!("No new block within {timeout:?} timeout");
                Ok(None)
            }
            Err(RecvTimeoutError::Disconnected) => {
                if self.shutdown_flag.is_on() {
                    Ok(None)
                } else {
                    Err(anyhow!("Indexer channel disconnected"))
                }
            }
        }
    }

    fn notify_consumers(&mut self, notification: BlockNotification) -> Result<()> {
        let (block, delivery_ack) = notification.into_parts();
        let result = self.deliver_to_consumers(block);
        let ack_result = match result.as_ref() {
            Ok(()) => Ok(()),
            Err(error) => Err(anyhow!("{error:#}")),
        };
        let _ = delivery_ack.send(ack_result);

        result
    }

    fn deliver_to_consumers(&mut self, new_block: RskBlockAndUncles) -> Result<()> {
        let hash = new_block.hash();
        let number = new_block.number();

        let response = FromServer::Block(new_block);

        for c_id in &self.consumers {
            trace!("Notifying consumer {c_id} about new block {number} ({hash})");

            self.msg_broker
                .send(&response, c_id)
                .context(format!("Sending block {number} ({hash}) to consumer {c_id}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::mpsc::Sender;
    use std::thread;
    use std::thread::{JoinHandle, sleep};

    use common_broker::broker::MockBrokerServerApi;
    use common_core::types::RskBlock;
    use common_dev::rsk_block_generator::{
        create_block_and_uncles, get_first_default_rsk_block, get_second_default_rsk_block,
    };

    use super::*;

    struct ClientRequest {
        id: Identifier,
        request: ToServer,
    }

    fn make_test_identifier(id: u8) -> Identifier {
        Identifier::new(format!("test_pubkey_hash_{id}"), id)
    }

    #[test]
    fn test_run_new_block_received_with_initial_consumer() {
        let client_id = make_test_identifier(1);

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let expected_block = get_first_default_rsk_block();

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker.expect_try_recv().returning(|| Ok(None));
        expect_send_block(&client_id, &expected_block, &[], &mut mock_broker);

        let mut notifier =
            Notifier::new_with_consumer(rx, mock_broker, shutdown_flag.clone(), client_id);
        notifier.check_period = Duration::from_millis(1);

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![RskBlockAndUncles::new_no_uncles(expected_block.clone())],
        );

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_new_block_received_no_consumers() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let expected_block = get_first_default_rsk_block();

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker.expect_try_recv().returning(|| Ok(None)); // no subscription message
        mock_broker.expect_send().never(); // nothing to send, no consumers yet

        let mut notifier = Notifier::new_for_tests(rx, mock_broker, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![RskBlockAndUncles::new_no_uncles(expected_block.clone())],
        );

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_no_blocks() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker
            .expect_try_recv()
            .returning(|| Ok(Some((ToServer::SubscribeBlocks, make_test_identifier(1))))); // subscription received
        mock_broker.expect_send().never(); // nothing to send, no blocks received yet

        let mut notifier = Notifier::new_for_tests(rx, mock_broker, shutdown_flag.clone());

        let handle_external_events = handle_external_events(tx, shutdown_flag, vec![]);

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_new_block_with_uncles() {
        let client_id = make_test_identifier(2);

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests =
            vec![ClientRequest { id: client_id.clone(), request: ToServer::SubscribeBlocks }];

        let (expected_block_1, expected_uncle_1, expected_block_2) = create_block_and_uncles();

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv(client_requests, &mut mock_broker_server);

        expect_send_block(&client_id, &expected_block_1, &[], &mut mock_broker_server);
        expect_send_block(
            &client_id,
            &expected_block_2,
            std::slice::from_ref(&expected_uncle_1),
            &mut mock_broker_server,
        );

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![
                RskBlockAndUncles::new_no_uncles(expected_block_1.clone()),
                RskBlockAndUncles::new(expected_block_2.clone(), vec![expected_uncle_1]),
            ],
        );

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_duplicate_subscribe_blocks_is_idempotent() {
        let client_id = make_test_identifier(2);

        let (_tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();
        let expected_block = get_first_default_rsk_block();

        let client_requests = vec![
            ClientRequest { id: client_id.clone(), request: ToServer::SubscribeBlocks },
            ClientRequest { id: client_id.clone(), request: ToServer::SubscribeBlocks },
        ];

        let mut mock_broker_server = MockBrokerServerApi::new();
        expect_try_recv(client_requests, &mut mock_broker_server);
        expect_send_block(&client_id, &expected_block, &[], &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag);

        notifier.update_consumers().expect("first subscribe should be accepted");
        notifier.update_consumers().expect("duplicate subscribe should be accepted");

        assert_eq!(1, notifier.consumers.len());

        notifier
            .notify_consumers(block_notification(RskBlockAndUncles::new_no_uncles(expected_block)))
            .expect("duplicate subscription should notify once");
    }

    #[test]
    fn test_run_new_block_received_with_multiple_consumers() {
        let client_id_1 = make_test_identifier(2);
        let client_id_2 = make_test_identifier(3);

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests = vec![
            ClientRequest { id: client_id_1.clone(), request: ToServer::SubscribeBlocks },
            ClientRequest { id: client_id_2.clone(), request: ToServer::SubscribeBlocks },
        ];

        let expected_block_1 = get_first_default_rsk_block();
        let expected_block_2 = get_second_default_rsk_block();

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv(client_requests, &mut mock_broker_server);

        expect_send_block(&client_id_1, &expected_block_1, &[], &mut mock_broker_server);
        expect_send_block(&client_id_2, &expected_block_1, &[], &mut mock_broker_server);
        expect_send_block(&client_id_1, &expected_block_2, &[], &mut mock_broker_server);
        expect_send_block(&client_id_2, &expected_block_2, &[], &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![
                RskBlockAndUncles::new_no_uncles(expected_block_1.clone()),
                RskBlockAndUncles::new_no_uncles(expected_block_2.clone()),
            ],
        );

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_unsubscribe() {
        let client_id_1 = make_test_identifier(2);
        let client_id_2 = make_test_identifier(3);

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests = vec![
            ClientRequest { id: client_id_1.clone(), request: ToServer::SubscribeBlocks },
            ClientRequest { id: client_id_2.clone(), request: ToServer::SubscribeBlocks },
            // should not receive blocks for this address
            ClientRequest { id: client_id_1.clone(), request: ToServer::UnsubscribeBlocks },
        ];

        let expected_block_1 = get_first_default_rsk_block();
        let expected_block_2 = get_second_default_rsk_block();

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv(client_requests, &mut mock_broker_server);

        expect_send_block(&client_id_2, &expected_block_1, &[], &mut mock_broker_server);
        expect_send_block(&client_id_2, &expected_block_2, &[], &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![
                RskBlockAndUncles::new_no_uncles(expected_block_1.clone()),
                RskBlockAndUncles::new_no_uncles(expected_block_2.clone()),
            ],
        );

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    fn expect_try_recv(
        client_requests: Vec<ClientRequest>,
        mock_broker_server: &mut MockBrokerServerApi<ToServer, FromServer>,
    ) {
        use std::collections::VecDeque;

        mock_broker_server.expect_try_recv().returning_st({
            let mut responses: VecDeque<_> =
                client_requests.into_iter().map(|coa| Ok(Some((coa.request, coa.id)))).collect();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn expect_send_block(
        dest: &Identifier,
        expected_block: &RskBlock,
        expected_uncles: &[RskBlock],
        mock_broker_server: &mut MockBrokerServerApi<ToServer, FromServer>,
    ) {
        mock_broker_server
            .expect_send()
            .withf({
                let expected_block = expected_block.clone(); // move into closure
                let dest = dest.clone();
                let expected_uncles = expected_uncles.to_owned();
                move |msg, dst| match msg {
                    FromServer::Block(bau) => {
                        *dst == dest
                            && *bau.block() == expected_block
                            && bau.uncles().iter().all(|u| expected_uncles.contains(u))
                    }
                    _ => false,
                }
            })
            .returning(|_, _| Ok(()))
            .once();
    }

    fn handle_external_events(
        tx: Sender<BlockNotification>,
        shutdown_flag: ShutdownFlag,
        blocks: Vec<RskBlockAndUncles>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            // give time for subscriptions to be processed
            sleep(Duration::from_millis(10));

            for block in blocks {
                tx.send(block_notification(block)).expect("Failed to send block");
            }

            // give time for messages to be processed
            sleep(Duration::from_millis(10));

            shutdown_flag.set();
        })
    }

    fn block_notification(block: RskBlockAndUncles) -> BlockNotification {
        let (delivery_ack, _delivery_ack_rx) = mpsc::channel();
        BlockNotification::new(block, delivery_ack)
    }
}
