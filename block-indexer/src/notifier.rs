use anyhow::{Context, Result, anyhow};
use common::constants::indexer::NOTIFIER_CHECK_PERIOD;
use common::msg_broker::broker::BrokerServerApi;
pub use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use common::types::RskBlock;
use log::{debug, info, trace, warn};
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

pub struct Notifier<BS: BrokerServerApi> {
    new_block_channel: mpsc::Receiver<BlockNotif>,
    msg_broker: BS,
    check_period: Duration,
    consumers: HashSet<u32>,
    shutdown_flag: ShutdownFlag,
}

#[derive(Debug)]
pub struct BlockNotif {
    pub block: RskBlock,
    pub uncles: Vec<RskBlock>,
}

impl<BS: BrokerServerApi> Notifier<BS> {
    pub fn new(
        indexer_receiver: mpsc::Receiver<BlockNotif>,
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

    #[cfg(test)]
    pub fn new_for_tests(
        indexer_receiver: mpsc::Receiver<BlockNotif>,
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

            if let Some(block) = self.wait_for_block(self.check_period)? {
                self.notify_consumers(block)?;
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

    fn wait_for_block(&mut self, timeout: Duration) -> Result<Option<BlockNotif>> {
        match self.new_block_channel.recv_timeout(timeout) {
            Ok(block) => {
                debug!("New block received by notifier {:?}", block);
                Ok(Some(block))
            }
            Err(RecvTimeoutError::Timeout) => {
                trace!("No new block within {:?} timeout", timeout);
                Ok(None)
            }
            Err(RecvTimeoutError::Disconnected) => match self.shutdown_flag.is_on() {
                true => Ok(None),
                false => Err(anyhow!("Indexer channel disconnected")),
            },
        }
    }

    fn notify_consumers(&mut self, new_block: BlockNotif) -> Result<()> {
        let hash = new_block.block.hash();
        let number = new_block.block.number();
        let response = BrokerResponses::Block(new_block.block, new_block.uncles);

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
    use common::types::BlockHash;
    use std::sync::mpsc;
    use std::sync::mpsc::Sender;
    use std::thread;
    use std::thread::{JoinHandle, sleep};
    use std::time::Duration;

    struct ClientRequest {
        id: u32,
        request: BrokerRequests,
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
            vec![BlockNotif {
                block: expected_block.clone(),
                uncles: vec![],
            }],
        );

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
    fn test_run_no_blocks() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker
            .expect_try_recv()
            .returning(|| Ok(Some((BrokerRequests::SubscribeBlocks, 1)))); // subscription received
        mock_broker.expect_send().never(); // nothing to send, no blocks received yet

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
    fn test_run_new_block_with_uncles() {
        let client_id = 2;

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let client_requests = vec![ClientRequest {
            id: client_id,
            request: BrokerRequests::SubscribeBlocks,
        }];

        let block_1_template = get_first_default_rsk_block();

        let expected_block_1 = create_block_from_template(
            &block_1_template,
            "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
            block_1_template.parent_hash(),
            vec![],
        );

        let expected_uncle_1 = create_block_from_template(
            &block_1_template,
            "0x3e5f9c2451b8efb4c1e3739816e44e4f0e9c25b2f9f6a57bdbf71e2df7c1b790",
            block_1_template.parent_hash(),
            vec![],
        );

        let expected_block_2 = create_block_from_template(
            &get_second_default_rsk_block(),
            "0x5c8a91d7ef0d46f3a65f1c345beab0cf56a8e065f2b762fe9b8e2d771fd42c83",
            expected_block_1.hash(),
            vec![expected_uncle_1.hash()],
        );

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv(client_requests, &mut mock_broker_server);

        expect_send_block(
            client_id,
            &expected_block_1,
            vec![],
            &mut mock_broker_server,
        );
        expect_send_block(
            client_id,
            &expected_block_2,
            vec![expected_uncle_1.clone()],
            &mut mock_broker_server,
        );

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![
                BlockNotif {
                    block: expected_block_1.clone(),
                    uncles: vec![],
                },
                BlockNotif {
                    block: expected_block_2.clone(),
                    uncles: vec![expected_uncle_1],
                },
            ],
        );

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

        expect_try_recv(client_requests, &mut mock_broker_server);

        expect_send_block(
            client_id_1,
            &expected_block_1,
            vec![],
            &mut mock_broker_server,
        );
        expect_send_block(
            client_id_2,
            &expected_block_1,
            vec![],
            &mut mock_broker_server,
        );
        expect_send_block(
            client_id_1,
            &expected_block_2,
            vec![],
            &mut mock_broker_server,
        );
        expect_send_block(
            client_id_2,
            &expected_block_2,
            vec![],
            &mut mock_broker_server,
        );

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![
                BlockNotif {
                    block: expected_block_1.clone(),
                    uncles: vec![],
                },
                BlockNotif {
                    block: expected_block_2.clone(),
                    uncles: vec![],
                },
            ],
        );

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

        expect_try_recv(client_requests, &mut mock_broker_server);

        expect_send_block(
            client_id_2,
            &expected_block_1,
            vec![],
            &mut mock_broker_server,
        );
        expect_send_block(
            client_id_2,
            &expected_block_2,
            vec![],
            &mut mock_broker_server,
        );

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![
                BlockNotif {
                    block: expected_block_1.clone(),
                    uncles: vec![],
                },
                BlockNotif {
                    block: expected_block_2.clone(),
                    uncles: vec![],
                },
            ],
        );

        let result = notifier.run();

        handle_external_events
            .join()
            .expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
            panic!("Run failed: {:?}", e);
        }
    }

    fn expect_try_recv(
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
        expected_uncles: Vec<RskBlock>,
        mock_broker_server: &mut MockBrokerServerApi,
    ) {
        mock_broker_server
            .expect_send()
            .withf({
                let expected_block = expected_block.clone(); // move into closure
                move |response, consumer_id| match response {
                    BrokerResponses::Block(actual_block, uncles) => {
                        *consumer_id == dest
                            && *actual_block == expected_block
                            && uncles.iter().all(|u| expected_uncles.contains(u))
                    }
                    _ => false,
                }
            })
            .returning(|_, _| Ok(()))
            .once();
    }

    fn handle_external_events(
        tx: Sender<BlockNotif>,
        shutdown_flag: ShutdownFlag,
        blocks: Vec<BlockNotif>,
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

    // TODO(iago) ticket for builder pattern for RskBlock in tests
    // TODO(iago) ticket for builder pattern for RskLog in tests

    fn create_block_from_template(
        template: &RskBlock,
        hash: &str,
        parent: BlockHash,
        uncles: Vec<BlockHash>,
    ) -> RskBlock {
        RskBlock::new(
            template.number(),
            BlockHash::try_from(hash).expect("Failed to parse hash"),
            parent,
            template.timestamp(),
            template.difficulty(),
            template.total_difficulty(),
            template.pow(),
            uncles,
        )
    }
}
