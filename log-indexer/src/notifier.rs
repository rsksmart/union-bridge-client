use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use common::constants::indexer::NOTIFIER_CHECK_PERIOD;
use common::msg_broker::broker::{Identifier, UnionBrokerServerApi};
use common::msg_broker::types::{FromServer, ToServer};
use common::shutdown_flag::ShutdownFlag;
use common::types::{Address, RskLog};
use log::{debug, error, info, trace, warn};

pub struct Notifier<BS: UnionBrokerServerApi> {
    new_log_channel: mpsc::Receiver<RskLog>,
    msg_broker: BS,
    contracts_with_consumers: HashMap<Address, HashSet<Identifier>>,
    check_period: Duration,
    shutdown_flag: ShutdownFlag,
}

impl<BS: UnionBrokerServerApi> Notifier<BS> {
    pub fn new(
        indexer_receiver: mpsc::Receiver<RskLog>,
        msg_broker: BS,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_log_channel: indexer_receiver,
            msg_broker,
            contracts_with_consumers: HashMap::new(),
            check_period: NOTIFIER_CHECK_PERIOD,
            shutdown_flag,
        }
    }

    #[cfg(test)]
    pub fn new_for_tests(
        indexer_receiver: mpsc::Receiver<RskLog>,
        msg_broker: BS,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            new_log_channel: indexer_receiver,
            msg_broker,
            check_period: Duration::from_millis(1),
            contracts_with_consumers: HashMap::new(),
            shutdown_flag,
        }
    }

    /// Run the notifier loop
    ///
    /// # Errors
    ///
    /// Returns an error if there's a failure in the message broker or channel communication
    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.shutdown_flag.is_on() {
                break;
            }

            self.update_consumers()?;

            if let Some(log) = self.wait_for_log(self.check_period)? {
                self.notify_consumers(log)?;
            }
        }

        info!("Shutdown requested, stopping notifier");

        Ok(())
    }

    fn update_consumers(&mut self) -> Result<()> {
        match self.msg_broker.try_recv()? {
            Some((ToServer::SubscribeLogs(event), consumer_id)) => {
                self.subscribe_consumer_to_contract(event, consumer_id);
            }
            Some((ToServer::UnsubscribeLogs(topic), consumer_id)) => {
                self.unsubscribe_consumer_from_contract(topic, &consumer_id);
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

    fn subscribe_consumer_to_contract(&mut self, address: Address, consumer_id: Identifier) {
        #[allow(clippy::collapsible_if)]
        if let Some(consumers) = self.contracts_with_consumers.get(&address) {
            if consumers.contains(&consumer_id) {
                warn!("Consumer {consumer_id} is already subscribed to {address}");
                return;
            }
        }

        info!("New consumer {consumer_id} subscribing to {address}");
        self.contracts_with_consumers.entry(address).or_default().insert(consumer_id);
    }

    fn unsubscribe_consumer_from_contract(&mut self, address: Address, consumer_id: &Identifier) {
        info!("Unsubscribing consumer {consumer_id}");
        if let Entry::Occupied(mut consumer) = self.contracts_with_consumers.entry(address) {
            consumer.get_mut().remove(consumer_id);
            let consumer_contracts = consumer.get();
            if consumer_contracts.is_empty() {
                consumer.remove_entry();
            }
        } else {
            trace!(
                "Unsubscribing consumer {consumer_id} that was not subscribed to contract {address}"
            );
        }
    }

    fn unsubscribe_consumer_from_all_contracts(&mut self, consumer_id: &Identifier) {
        info!("Unsubscribing consumer {consumer_id} from all contracts");
        self.contracts_with_consumers.retain(|_, consumers| {
            consumers.remove(consumer_id);
            !consumers.is_empty()
        });
    }

    fn wait_for_log(&mut self, timeout: Duration) -> Result<Option<RskLog>> {
        match self.new_log_channel.recv_timeout(timeout) {
            Ok(log) => {
                trace!("New log received by notifier {log:?}");
                Ok(Some(log))
            }
            Err(RecvTimeoutError::Timeout) => {
                trace!("No new log within {timeout:?} timeout");
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

    fn notify_consumers(&mut self, new_log: RskLog) -> Result<()> {
        let address: Address = new_log.info().address();

        let topics0 = new_log.event().topics().first().map_or_else(
            || {
                error!("Log has no topics, using NoTopic for selector");
                "NoTopic".to_string()
            },
            ToString::to_string,
        );

        let selector = format!("{topics0} @ {address}");

        let response = FromServer::Log(new_log);

        if let Some(consumers_for_contract) = self.contracts_with_consumers.get(&address) {
            for c_id in consumers_for_contract {
                trace!("Notifying {selector} to consumer {c_id}");

                self.msg_broker
                    .send(&response, c_id)
                    .context(format!("Sending {selector} to consumer {c_id}"))?;
            }
        } else {
            debug!("No consumers for contract @ {address}");
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

    use common::msg_broker::broker::MockBrokerServerApi;
    use common::test_utils::rsk_log_generator::FakeLogGenerator;
    use common::test_utils::rsk_utils::generate_fake_address;

    use super::*;

    struct ClientRequest {
        id: Identifier,
        request: ToServer,
    }

    fn make_test_identifier(id: u8) -> Identifier {
        Identifier::new(format!("test_pubkey_hash_{id}"), id)
    }

    #[test]
    fn test_run_new_log_received_no_consumers() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let address = generate_fake_address(1);
        let expected_log =
            FakeLogGenerator::new().generate_log("Transfer(address,address,uint256)", address);

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker.expect_try_recv().returning(|| Ok(None)); // no subscription message
        mock_broker.expect_send().never(); // nothing to send, no consumers yet

        let mut notifier = Notifier::new_for_tests(rx, mock_broker, shutdown_flag.clone());

        let handle_external_events = handle_external_events(tx, shutdown_flag, vec![expected_log]);

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_new_log_received_no_consumer_for_address() {
        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let address = generate_fake_address(1);
        let expected_log =
            FakeLogGenerator::new().generate_log("Transfer(address,address,uint256)", address);

        let mut mock_broker = MockBrokerServerApi::new();
        mock_broker.expect_try_recv().returning(|| {
            Ok(Some((ToServer::SubscribeLogs(generate_fake_address(2)), make_test_identifier(1))))
        }); // subscribe for a different address
        mock_broker.expect_send().never(); // nothing to send, no consumers yet for that address

        let mut notifier = Notifier::new_for_tests(rx, mock_broker, shutdown_flag.clone());

        let handle_external_events = handle_external_events(tx, shutdown_flag, vec![expected_log]);

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_new_log_received_with_consumer() {
        let client_id = make_test_identifier(2);

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let address_1 = generate_fake_address(1);

        let client_requests = vec![ClientRequest {
            id: client_id.clone(),
            request: ToServer::SubscribeLogs(address_1),
        }];

        let expected_log_1 =
            FakeLogGenerator::new().generate_log("Transfer(address,address,uint256)", address_1);
        let expected_log_2 =
            FakeLogGenerator::new().generate_log("Withdraw(address,address,uint256)", address_1);

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv_subscribe(client_requests, &mut mock_broker_server);

        expect_send_log(&client_id, &expected_log_1, &mut mock_broker_server);
        expect_send_log(&client_id, &expected_log_2, &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events =
            handle_external_events(tx, shutdown_flag, vec![expected_log_1, expected_log_2]);

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    #[test]
    fn test_run_new_log_received_with_multiple_consumers() {
        let client_id_1 = make_test_identifier(2);
        let client_id_2 = make_test_identifier(3);

        let (tx, rx) = mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();

        let address_1 = generate_fake_address(1);
        let address_2 = generate_fake_address(2);

        let client_requests = vec![
            ClientRequest { id: client_id_1.clone(), request: ToServer::SubscribeLogs(address_1) },
            ClientRequest { id: client_id_2.clone(), request: ToServer::SubscribeLogs(address_1) },
        ];

        let expected_log_1 =
            FakeLogGenerator::new().generate_log("Transfer(address,address,uint256)", address_1);
        let expected_log_2 =
            FakeLogGenerator::new().generate_log("Withdraw(address,address,uint256)", address_1);
        let not_expected_log_3 =
            FakeLogGenerator::new().generate_log("Renew(address,address,uint256)", address_2);

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv_subscribe(client_requests, &mut mock_broker_server);

        expect_send_log(&client_id_1, &expected_log_1, &mut mock_broker_server);
        expect_send_log(&client_id_2, &expected_log_1, &mut mock_broker_server);
        expect_send_log(&client_id_1, &expected_log_2, &mut mock_broker_server);
        expect_send_log(&client_id_2, &expected_log_2, &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![expected_log_1, expected_log_2, not_expected_log_3],
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

        let address_1 = generate_fake_address(1);

        let client_requests = vec![
            ClientRequest { id: client_id_1.clone(), request: ToServer::SubscribeLogs(address_1) },
            ClientRequest { id: client_id_2.clone(), request: ToServer::SubscribeLogs(address_1) },
            // should not receive logs for this address
            ClientRequest {
                id: client_id_1.clone(),
                request: ToServer::UnsubscribeLogs(address_1),
            },
        ];

        let expected_log_1_for_2 =
            FakeLogGenerator::new().generate_log("Transfer(address,address,uint256)", address_1);
        let expected_log_2_for_2 =
            FakeLogGenerator::new().generate_log("Withdraw(address,address,uint256)", address_1);

        let mut mock_broker_server = MockBrokerServerApi::new();

        expect_try_recv_subscribe(client_requests, &mut mock_broker_server);

        expect_send_log(&client_id_2, &expected_log_1_for_2, &mut mock_broker_server);
        expect_send_log(&client_id_2, &expected_log_2_for_2, &mut mock_broker_server);

        let mut notifier = Notifier::new_for_tests(rx, mock_broker_server, shutdown_flag.clone());

        let handle_external_events = handle_external_events(
            tx,
            shutdown_flag,
            vec![expected_log_1_for_2, expected_log_2_for_2],
        );

        let result = notifier.run();

        handle_external_events.join().expect("Failed to join shutdown handle");

        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
            panic!("Run failed: {e:?}");
        }
    }

    fn expect_try_recv_subscribe(
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

    fn expect_send_log(
        dest: &Identifier,
        expected_log: &RskLog,
        mock_broker_server: &mut MockBrokerServerApi<ToServer, FromServer>,
    ) {
        mock_broker_server
            .expect_send()
            .withf({
                let expected_log = expected_log.clone(); // move into closure
                let dest = dest.clone();
                move |msg, dst| match msg {
                    FromServer::Log(actual_log) => *dst == dest && *actual_log == expected_log,
                    _ => false,
                }
            })
            .returning(|_, _| Ok(()))
            .once();
    }

    fn handle_external_events(
        tx: Sender<RskLog>,
        shutdown_flag: ShutdownFlag,
        logs: Vec<RskLog>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            // give time for subscriptions to be processed
            sleep(Duration::from_millis(10));

            for log in logs {
                tx.send(log).expect("Failed to send log");
            }

            // give time for messages to be processed
            sleep(Duration::from_millis(10));

            shutdown_flag.set();
        })
    }
}
