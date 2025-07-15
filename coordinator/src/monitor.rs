use crate::types::{EventDecoder, RskPegManagerEvents};
use anyhow::{Context, Result, bail};
use common::msg_broker::bitvmx_types::OutgoingBitVMXApiMessages;
use common::{
    msg_broker::{
        broker::BitVmxBrokerClientApi,
        broker::{BROKER_SERVER_ID, BrokerError, UnionBrokerClientApi},
        types::{FromServer, ToServer},
    },
    types::{Address, RskBlockAndUncles},
};
use log::{debug, info, trace};
use std::rc::Rc;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait MonitorApi {
    fn start_event_monitoring(&mut self) -> Result<()>;
    fn start_block_monitoring(&mut self) -> Result<()>;
    fn start_bitvmx_monitoring(&mut self) -> Result<()>;
    fn try_event(&mut self) -> Result<Option<RskPegManagerEvents>>;
    fn try_block(&mut self) -> Result<Option<RskBlockAndUncles>>;
    fn try_bitvmx_event(&mut self) -> Result<Option<OutgoingBitVMXApiMessages>>;
    fn cancel_event_monitoring(&mut self) -> Result<()>;
    fn cancel_block_monitoring(&mut self) -> Result<()>;
    fn cancel_bitvmx_monitoring(&mut self) -> Result<()>;
}

pub struct Monitor<UBC, BBC>
where
    UBC: UnionBrokerClientApi,
    BBC: BitVmxBrokerClientApi,
{
    log_broker: UBC,
    block_broker: UBC,
    bitvmx_broker: Rc<BBC>,
    event_decoder: EventDecoder,
    peg_manager_addresses: Vec<Address>,
    block_monitoring_active: bool,
    log_monitoring_active: bool,
    bitvmx_monitoring_active: bool,
}

impl<UBC, BBC> MonitorApi for Monitor<UBC, BBC>
where
    UBC: UnionBrokerClientApi,
    BBC: BitVmxBrokerClientApi,
{
    fn start_event_monitoring(&mut self) -> Result<()> {
        self.start_event_monitoring()
    }

    fn start_block_monitoring(&mut self) -> Result<()> {
        self.start_block_monitoring()
    }

    fn start_bitvmx_monitoring(&mut self) -> Result<()> {
        self.start_bitvmx_monitoring()
    }

    fn try_event(&mut self) -> Result<Option<RskPegManagerEvents>> {
        self.try_event()
    }

    fn try_block(&mut self) -> Result<Option<RskBlockAndUncles>> {
        self.try_block()
    }

    fn try_bitvmx_event(&mut self) -> Result<Option<OutgoingBitVMXApiMessages>> {
        self.try_bitvmx_event()
    }

    fn cancel_event_monitoring(&mut self) -> Result<()> {
        self.cancel_event_monitoring()
    }

    fn cancel_block_monitoring(&mut self) -> Result<()> {
        self.cancel_block_monitoring()
    }

    fn cancel_bitvmx_monitoring(&mut self) -> Result<()> {
        self.cancel_bitvmx_monitoring()
    }
}

impl<UBC, BBC> Monitor<UBC, BBC>
where
    UBC: UnionBrokerClientApi,
    BBC: BitVmxBrokerClientApi,
{
    pub fn new(
        log_broker: UBC,
        block_broker: UBC,
        bitvmx_broker: Rc<BBC>,
        peg_manager_addresses: Vec<Address>,
    ) -> Self {
        Self {
            log_broker,
            block_broker,
            bitvmx_broker,
            event_decoder: EventDecoder::new(),
            peg_manager_addresses,
            block_monitoring_active: false,
            log_monitoring_active: false,
            bitvmx_monitoring_active: false,
        }
    }

    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - retries, reconnects, etc
    pub fn start_event_monitoring(&mut self) -> Result<()> {
        if self.log_monitoring_active {
            bail!("Start Log monitoring requested, but it was already active");
        }

        // clean up a potential remaining connection
        self.request_cancel_event_monitoring()
            .context("Cleaning up potentially stalled log connection")?;

        let addresses = self.peg_manager_addresses.clone();
        for addr in addresses {
            let result = self
                .send_to_log_broker(ToServer::SubscribeLogs(addr))
                .context("Broker error on SubscribeLogs")?;
            if !result {
                bail!("Broker could not deliver SubscribeLogs for {}", addr);
            }
        }

        self.log_monitoring_active = true;

        Ok(())
    }

    pub fn start_block_monitoring(&mut self) -> Result<()> {
        if self.block_monitoring_active {
            bail!("Start Block monitoring requested, but it was already active");
        }

        // clean up a potential remaining connection
        self.request_cancel_block_monitoring()
            .context("Cleaning up potentially stalled block connection")?;

        info!("Starting Block monitoring");

        let result = self
            .send_to_block_broker(ToServer::SubscribeBlocks)
            .context("Broker error on SubscribeBlocks")?;

        if !result {
            bail!("Broker could not deliver SubscribeBlocks")
        }

        self.block_monitoring_active = true;

        Ok(())
    }

    pub fn start_bitvmx_monitoring(&mut self) -> Result<()> {
        if self.bitvmx_monitoring_active {
            bail!("Start BitVMX monitoring requested, but it was already active");
        }

        info!("Starting BitVMX monitoring");

        self.bitvmx_monitoring_active = true;

        Ok(())
    }

    pub fn try_event(&mut self) -> Result<Option<RskPegManagerEvents>> {
        if !self.log_monitoring_active {
            bail!("Log monitoring is not active");
        }

        match self.log_broker.try_recv()? {
            Some(FromServer::Log(log)) => {
                info!("Received new Log {:?}", log);
                let event: RskPegManagerEvents = self.event_decoder.decode(log);
                Ok(Some(event))
            }
            Some(br) => {
                bail!("Unexpected response type from Log Notifier {:?}", br)
            }
            None => {
                trace!("No messages from Log broker");
                Ok(None)
            }
        }
    }

    pub fn try_block(&mut self) -> Result<Option<RskBlockAndUncles>> {
        if !self.block_monitoring_active {
            bail!("Block monitoring is not active");
        }

        // TODO(Jira) do not simply fail on broker error, do some retries - https://rsklabs.atlassian.net/browse/UB-132
        match self.block_broker.try_recv()? {
            Some(FromServer::Block(bau)) => {
                debug!("Received new Block {:?}", bau);
                Ok(Some(bau))
            }
            Some(other) => bail!("Unexpected response type from Block broker: {:?}", other),
            None => {
                trace!("No messages from Block broker");
                Ok(None)
            }
        }
    }

    pub fn try_bitvmx_event(&mut self) -> Result<Option<OutgoingBitVMXApiMessages>> {
        if !self.bitvmx_monitoring_active {
            bail!("BitVMX monitoring is not active");
        }

        match self.bitvmx_broker.try_recv()? {
            Some(response) => {
                debug!("Received BitVMX response: {:?}", response);
                Ok(Some(response))
            }
            None => {
                trace!("No messages from BitVMX broker");
                Ok(None)
            }
        }
    }

    pub fn cancel_event_monitoring(&mut self) -> Result<()> {
        if !self.log_monitoring_active {
            bail!("Cancel Log monitoring requested, but it was not active");
        }

        if !self.request_cancel_event_monitoring()? {
            bail!("Broker could not deliver UnsubscribeLogs")
        }

        self.log_monitoring_active = false;

        Ok(())
    }

    pub fn cancel_block_monitoring(&mut self) -> Result<()> {
        if !self.block_monitoring_active {
            bail!("Cancel Block monitoring requested, but it was not active");
        };

        info!("Cancelling Block monitoring");

        if !self.request_cancel_block_monitoring()? {
            bail!("Broker could not deliver UnsubscribeBlocks")
        }

        self.block_monitoring_active = false;

        Ok(())
    }

    pub fn cancel_bitvmx_monitoring(&mut self) -> Result<()> {
        if !self.bitvmx_monitoring_active {
            bail!("Cancel BitVMX monitoring requested, but it was not active");
        }

        self.bitvmx_monitoring_active = false;

        Ok(())
    }

    fn request_cancel_event_monitoring(&mut self) -> Result<bool> {
        let mut result = true;

        let addresses = self.peg_manager_addresses.clone();
        for addr in addresses {
            result = result
                && self
                    .send_to_log_broker(ToServer::UnsubscribeLogs(addr))
                    .context("Broker error on UnsubscribeLogs")?;
            if !result {
                bail!("Broker could not deliver UnsubscribeLogs for {}", addr);
            }
        }

        Ok(result)
    }

    fn request_cancel_block_monitoring(&mut self) -> Result<bool> {
        self.send_to_block_broker(ToServer::UnsubscribeBlocks)
            .context("Broker error on UnsubscribeBlocks")
    }

    fn send_to_log_broker(&mut self, request: ToServer) -> Result<bool, BrokerError> {
        self.log_broker.send(BROKER_SERVER_ID, request)
    }

    fn send_to_block_broker(&mut self, request: ToServer) -> Result<bool, BrokerError> {
        self.block_broker.send(BROKER_SERVER_ID, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use common::msg_broker::bitvmx_types::IncomingBitVMXApiMessages;
    use common::test_utils::rsk_block_generator::{
        create_block_from_template, get_first_default_rsk_block, get_second_default_rsk_block,
        get_third_default_rsk_block,
    };
    use common::{
        msg_broker::{
            broker::{BROKER_SERVER_ID, MockBrokerClientApi},
            types::ToServer,
        },
        test_utils::{
            rsk_block_generator::create_block_and_uncles, rsk_log_generator::FakeLogGenerator,
        },
    };
    use mockall::predicate::*;

    #[test]
    fn test_try_block_handles_wrong_order_blocks() {
        let mut block_broker = MockBrokerClientApi::new();
        let template_block1 = get_first_default_rsk_block();
        let template_block2 = get_second_default_rsk_block();
        let template_block3 = get_third_default_rsk_block();

        let block1 = create_block_from_template(
            &template_block1,
            "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
            template_block1.parent_hash(),
            vec![],
        );
        let block2 = create_block_from_template(
            &template_block2,
            "0x5c8a91d7ef0d46f3a65f1c345beab0cf56a8e065f2b762fe9b8e2d771fd42c83",
            block1.hash(),
            vec![],
        );
        let block3 = create_block_from_template(
            &template_block3,
            "0x3e5f9c2451b8efb4c1e3739816e44e4f0e9c25b2f9f6a57bdbf71e2df7c1b790",
            block2.hash(),
            vec![],
        );

        // Mock the broker to return blocks in wrong order
        block_broker.expect_try_recv().times(3).returning({
            let b3 = block3.clone();
            let b2 = block2.clone();
            let b1 = block1.clone();
            let mut call_count = 0;
            move || {
                call_count += 1;
                match call_count {
                    1 => Ok(Some(FromServer::Block(RskBlockAndUncles::new(
                        b3.clone(),
                        vec![],
                    )))),
                    2 => Ok(Some(FromServer::Block(RskBlockAndUncles::new(
                        b2.clone(),
                        vec![],
                    )))),
                    3 => Ok(Some(FromServer::Block(RskBlockAndUncles::new(
                        b1.clone(),
                        vec![],
                    )))),
                    _ => Ok(None),
                }
            }
        });

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.block_monitoring_active = true;

        let result1 = monitor.try_block().expect("Failed to receive first block");
        assert_eq!(
            result1,
            Some(RskBlockAndUncles::new(block3.clone(), vec![]))
        );
        let result2 = monitor.try_block().expect("Failed to receive second block");
        assert_eq!(
            result2,
            Some(RskBlockAndUncles::new(block2.clone(), vec![]))
        );
        let result3 = monitor.try_block().expect("Failed to receive third block");
        assert_eq!(
            result3,
            Some(RskBlockAndUncles::new(block1.clone(), vec![]))
        );

        assert_eq!(block1.hash(), block2.parent_hash());
        assert_eq!(block2.hash(), block3.parent_hash());

        // Verify blocks were received in wrong order
        assert_eq!(result1.as_ref().unwrap().block().hash(), block3.hash());
        assert_eq!(result2.as_ref().unwrap().block().hash(), block2.hash());
        assert_eq!(result3.as_ref().unwrap().block().hash(), block1.hash());
    }

    #[test]
    fn test_start_event_monitoring_success() {
        let address_1 = get_fake_address_1();
        let address_2 = get_fake_address_2();

        let mut log_broker = MockBrokerClientApi::new();
        expect_unsubscribe_logs(&mut log_broker, address_1);
        expect_unsubscribe_logs(&mut log_broker, address_2);
        expect_subscribe_logs(&mut log_broker, address_1);
        expect_subscribe_logs(&mut log_broker, address_2);

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![address_1, address_2],
        );

        assert!(monitor.start_event_monitoring().is_ok());
        assert!(monitor.log_monitoring_active);
    }

    #[test]
    fn test_start_event_monitoring_fails_on_broker_error() {
        let address_1 = get_fake_address_1();

        let mut log_broker = MockBrokerClientApi::new();
        expect_unsubscribe_logs(&mut log_broker, address_1);

        log_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), function(move |req: &ToServer| {
                matches!(req, ToServer::SubscribeLogs(a) if *a == address_1)
            }))
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![address_1],
        );
        let err = monitor.start_event_monitoring();
        assert!(err.is_err());
        assert!(
            err.as_ref()
                .unwrap_err()
                .to_string()
                .contains("Broker error on SubscribeLogs")
        );
    }

    #[test]
    fn test_start_event_monitoring_fails_if_already_active() {
        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.log_monitoring_active = true;
        let err = monitor.start_event_monitoring();
        assert!(err.is_err());
        assert!(
            err.as_ref()
                .unwrap_err()
                .to_string()
                .contains("already active")
        );
    }

    #[test]
    fn test_start_block_monitoring_success() {
        let mut block_broker = MockBrokerClientApi::new();
        expect_unsubscribe_blocks(&mut block_broker, 1);
        expect_subscribe_blocks(&mut block_broker, 1);

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        assert!(monitor.start_block_monitoring().is_ok());
        assert!(monitor.block_monitoring_active);
    }

    #[test]
    fn test_start_block_monitoring_fails_on_broker_error() {
        let mut block_broker = MockBrokerClientApi::new();
        expect_unsubscribe_blocks(&mut block_broker, 1);

        block_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &ToServer| matches!(req, ToServer::SubscribeBlocks)),
            )
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        let err = monitor.start_block_monitoring();
        assert!(err.is_err());
        assert!(
            err.as_ref()
                .unwrap_err()
                .to_string()
                .contains("Broker error on SubscribeBlocks")
        );
    }

    #[test]
    fn test_start_block_monitoring_fails_if_already_active() {
        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.block_monitoring_active = true;
        let err = monitor.start_block_monitoring();
        assert!(err.is_err());
    }

    #[test]
    fn test_start_bitvmx_monitoring_fails_if_already_active() {
        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.bitvmx_monitoring_active = true;
        let err = monitor.start_bitvmx_monitoring();
        assert!(err.is_err());
    }

    #[test]
    fn test_try_event_returns_some() {
        let log = FakeLogGenerator::new()
            .generate_log("Transfer(address,address,uint256", get_fake_address_1());

        let event_decoder = EventDecoder::new();

        let expected_event: RskPegManagerEvents = event_decoder.decode(log.clone());

        let mut log_broker = MockBrokerClientApi::new();
        log_broker
            .expect_try_recv()
            .return_once(move || Ok(Some(FromServer::Log(log))));

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.log_monitoring_active = true;

        let result = monitor.try_event().expect("Failed to receive event");
        assert_eq!(result, Some(expected_event));
    }

    #[test]
    fn test_try_event_returns_none() {
        let mut log_broker = MockBrokerClientApi::new();
        log_broker.expect_try_recv().return_once(move || Ok(None));

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.log_monitoring_active = true;

        let result = monitor.try_event().expect("Failed to receive event");
        assert_eq!(result, None);
    }

    #[test]
    fn test_try_block_returns_some() {
        let mut block_broker = MockBrokerClientApi::new();

        let (expected_block_1, expected_uncle_1, _) = create_block_and_uncles();

        block_broker.expect_try_recv().return_once({
            let block = expected_block_1.clone();
            let uncle = expected_uncle_1.clone();
            move || {
                Ok(Some(FromServer::Block(RskBlockAndUncles::new(
                    block,
                    vec![uncle],
                ))))
            }
        });

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.block_monitoring_active = true;

        let result = monitor.try_block().expect("Failed to receive block");
        assert_eq!(
            result,
            Some(RskBlockAndUncles::new(
                expected_block_1,
                vec![expected_uncle_1]
            ))
        );
    }

    #[test]
    fn test_try_block_returns_none() {
        let mut block_broker = MockBrokerClientApi::new();
        block_broker.expect_try_recv().return_once(move || Ok(None));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.block_monitoring_active = true;

        let result = monitor.try_block().expect("Failed to receive block");
        assert_eq!(result, None);
    }

    #[test]
    fn test_try_bitvmx_event_returns_some() {
        let value = OutgoingBitVMXApiMessages::Pong();
        let mock_value = value.clone();
        let mut bitvmx_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        bitvmx_broker
            .expect_try_recv()
            .return_once(move || Ok(Some(mock_value)));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::<ToServer, FromServer>::new(),
            MockBrokerClientApi::<ToServer, FromServer>::new(),
            Rc::new(bitvmx_broker),
            vec![get_fake_address_1()],
        );
        monitor.bitvmx_monitoring_active = true;

        let result = monitor
            .try_bitvmx_event()
            .expect("Failed to receive BitVMX event");
        assert!(matches!(result, Some(OutgoingBitVMXApiMessages::Pong())));
    }

    #[test]
    fn test_try_bitvmx_event_returns_none() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        bitvmx_broker
            .expect_try_recv()
            .return_once(move || Ok(None));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            Rc::new(bitvmx_broker),
            vec![get_fake_address_1()],
        );
        monitor.bitvmx_monitoring_active = true;

        let result = monitor
            .try_bitvmx_event()
            .expect("Failed to receive BitVMX event");
        assert!(matches!(result, None));
    }

    #[test]
    fn test_cancel_event_monitoring_success() {
        let address_1 = get_fake_address_1();
        let address_2 = get_fake_address_2();

        let mut log_broker = MockBrokerClientApi::new();
        expect_unsubscribe_logs(&mut log_broker, address_1);
        expect_unsubscribe_logs(&mut log_broker, address_2);

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            Rc::new(MockBrokerClientApi::new()),
            vec![address_1, address_2],
        );
        monitor.log_monitoring_active = true;

        assert!(monitor.cancel_event_monitoring().is_ok());
        assert!(!monitor.log_monitoring_active);
    }

    #[test]
    fn test_cancel_block_monitoring_success() {
        let mut block_broker = MockBrokerClientApi::new();
        expect_unsubscribe_blocks(&mut block_broker, 1);

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            Rc::new(MockBrokerClientApi::new()),
            vec![get_fake_address_1()],
        );
        monitor.block_monitoring_active = true;

        assert!(monitor.cancel_block_monitoring().is_ok());
        assert!(!monitor.block_monitoring_active);
    }

    #[test]
    fn test_cancel_bitvmx_monitoring_success() {
        let bitvmx_broker = MockBrokerClientApi::new();

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            Rc::new(bitvmx_broker),
            vec![get_fake_address_1()],
        );
        monitor.bitvmx_monitoring_active = true;

        assert!(monitor.cancel_bitvmx_monitoring().is_ok());
        assert!(!monitor.bitvmx_monitoring_active);
    }

    fn expect_subscribe_logs(
        log_broker: &mut MockBrokerClientApi<ToServer, FromServer>,
        addr: Address,
    ) {
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(
                    move |req: &ToServer| matches!(req, ToServer::SubscribeLogs(a) if *a == addr),
                ),
            )
            .return_once(|_, _| Ok(true));
    }

    fn expect_subscribe_blocks(
        block_broker: &mut MockBrokerClientApi<ToServer, FromServer>,
        times: usize,
    ) {
        block_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &ToServer| matches!(req, ToServer::SubscribeBlocks)),
            )
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn expect_unsubscribe_logs(
        log_broker: &mut MockBrokerClientApi<ToServer, FromServer>,
        addr: Address,
    ) {
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(
                    move |req: &ToServer| matches!(req, ToServer::UnsubscribeLogs(a) if *a == addr),
                ),
            )
            .return_once(|_, _| Ok(true));
    }

    fn expect_unsubscribe_blocks(
        block_broker: &mut MockBrokerClientApi<ToServer, FromServer>,
        times: usize,
    ) {
        block_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &ToServer| matches!(req, ToServer::UnsubscribeBlocks)),
            )
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn get_fake_address_1() -> Address {
        Address::try_from("0x0165878A594ca255338adfa4d48449f69242Eb8F").expect("Invalid address")
    }

    fn get_fake_address_2() -> Address {
        Address::try_from("0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761").expect("Invalid address")
    }
}
