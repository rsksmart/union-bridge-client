use crate::types::{EventDecoder, RskPegManagerEvents};
use anyhow::{Context, Result, bail};
use common::{
    msg_broker::{
        broker::{BROKER_SERVER_ID, BrokerClientApi, BrokerError},
        types::{BrokerRequests, BrokerResponses},
    },
    types::{Address, RskBlockAndUncles},
};
use log::{debug, info, trace};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait MonitorApi {
    fn start_event_monitoring(&mut self) -> Result<()>;
    fn start_block_monitoring(&mut self) -> Result<()>;
    fn start_bitvmx_monitoring(&mut self) -> Result<()>;
    fn try_event(&mut self) -> Result<Option<RskPegManagerEvents>>;
    fn try_block(&mut self) -> Result<Option<RskBlockAndUncles>>;
    fn try_bitvmx_event(&mut self) -> Result<Option<BrokerResponses>>;
    fn cancel_event_monitoring(&mut self) -> Result<()>;
    fn cancel_block_monitoring(&mut self) -> Result<()>;
    fn cancel_bitvmx_monitoring(&mut self) -> Result<()>;
}

pub struct Monitor<T: BrokerClientApi> {
    log_broker: T,
    block_broker: T,
    bitvmx_broker: T,
    event_decoder: EventDecoder,
    peg_manager_address: Address,
    block_monitoring_active: bool,
    log_monitoring_active: bool,
    bitvmx_monitoring_active: bool,
}

impl<T: BrokerClientApi> MonitorApi for Monitor<T> {
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

    fn try_bitvmx_event(&mut self) -> Result<Option<BrokerResponses>> {
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

impl<T: BrokerClientApi> Monitor<T> {
    pub fn new(
        log_broker: T,
        block_broker: T,
        bitvmx_broker: T,
        peg_manager_address: Address,
    ) -> Self {
        Self {
            log_broker,
            block_broker,
            bitvmx_broker,
            event_decoder: EventDecoder::new(),
            peg_manager_address,
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

        info!("Starting event monitoring for {}", self.peg_manager_address);

        let result = self
            .send_to_log_broker(BrokerRequests::SubscribeLogs(self.peg_manager_address))
            .context("Broker error on SubscribeLogs")?;

        if !result {
            bail!("Broker could not deliver SubscribeLogs")
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
            .send_to_block_broker(BrokerRequests::SubscribeBlocks)
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

        // clean up a potential remaining connection
        self.request_cancel_bitvmx_monitoring()
            .context("Cleaning up stalled bitvmx connection")?;

        info!("Starting BitVMX monitoring");

        let result = self
            .send_to_bitvmx_broker(BrokerRequests::SubscribeBitVMX)
            .context("Broker error on SubscribeBitVMX")?;

        if !result {
            bail!("Broker could not deliver SubscribeBitVMX")
        }

        self.bitvmx_monitoring_active = true;

        Ok(())
    }

    pub fn try_event(&mut self) -> Result<Option<RskPegManagerEvents>> {
        if !self.log_monitoring_active {
            bail!("Log monitoring is not active");
        }

        match self.log_broker.try_recv()? {
            Some(BrokerResponses::Log(log)) => {
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
            Some(BrokerResponses::Block(bau)) => {
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

    pub fn try_bitvmx_event(&mut self) -> Result<Option<BrokerResponses>> {
        if !self.bitvmx_monitoring_active {
            bail!("BitVMX monitoring is not active");
        }

        match self.bitvmx_broker.try_recv()? {
            Some(response) => {
                info!("Received BitVMX response: {:?}", response);
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

        if !self.request_cancel_bitvmx_monitoring()? {
            bail!("Broker could not deliver UnsubscribeBitVMX")
        }

        self.bitvmx_monitoring_active = false;

        Ok(())
    }

    fn request_cancel_event_monitoring(&mut self) -> Result<bool> {
        self.send_to_log_broker(BrokerRequests::UnsubscribeLogs(self.peg_manager_address))
            .context("Broker error on UnsubscribeLogs")
    }

    fn request_cancel_block_monitoring(&mut self) -> Result<bool> {
        self.send_to_block_broker(BrokerRequests::UnsubscribeBlocks)
            .context("Broker error on UnsubscribeBlocks")
    }

    fn request_cancel_bitvmx_monitoring(&mut self) -> Result<bool> {
        self.send_to_bitvmx_broker(BrokerRequests::UnsubscribeBitVMX)
            .context("Broker error on UnsubscribeBitVMX")
    }

    fn send_to_log_broker(&mut self, request: BrokerRequests) -> Result<bool, BrokerError> {
        self.log_broker.send(BROKER_SERVER_ID, request)
    }

    fn send_to_block_broker(&mut self, request: BrokerRequests) -> Result<bool, BrokerError> {
        self.block_broker.send(BROKER_SERVER_ID, request)
    }

    fn send_to_bitvmx_broker(&mut self, request: BrokerRequests) -> Result<bool, BrokerError> {
        self.bitvmx_broker.send(BROKER_SERVER_ID, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use common::{
        msg_broker::{
            broker::{BROKER_SERVER_ID, MockBrokerClientApi},
            types::BrokerRequests,
        },
        test_utils::{
            rsk_block_generator::create_block_and_uncles, rsk_log_generator::FakeLogGenerator,
        },
    };
    use mockall::predicate::*;
    use serde_json::json;

    #[test]
    fn test_start_event_monitoring_success() {
        let mut log_broker = MockBrokerClientApi::new();
        expect_unsubscribe_logs(&mut log_broker, 1);
        expect_subscribe_logs(&mut log_broker, 1);

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            get_fake_address(),
        );

        assert!(monitor.start_event_monitoring().is_ok());
        assert!(monitor.log_monitoring_active);
    }

    #[test]
    fn test_start_event_monitoring_fails_on_broker_error() {
        let mut log_broker = MockBrokerClientApi::new();
        expect_unsubscribe_logs(&mut log_broker, 1);

        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::SubscribeLogs(get_fake_address())),
            )
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            get_fake_address(),
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
            MockBrokerClientApi::new(),
            get_fake_address(),
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
            MockBrokerClientApi::new(),
            get_fake_address(),
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
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::SubscribeBlocks))
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            MockBrokerClientApi::new(),
            get_fake_address(),
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
            MockBrokerClientApi::new(),
            get_fake_address(),
        );
        monitor.block_monitoring_active = true;
        let err = monitor.start_block_monitoring();
        assert!(err.is_err());
    }

    #[test]
    fn test_start_bitvmx_monitoring_success() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_unsubscribe_bitvmx(&mut bitvmx_broker, 1);
        expect_subscribe_bitvmx(&mut bitvmx_broker, 1);

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            bitvmx_broker,
            get_fake_address(),
        );

        assert!(monitor.start_bitvmx_monitoring().is_ok());
        assert!(monitor.bitvmx_monitoring_active);
    }

    #[test]
    fn test_start_bitvmx_monitoring_fails_on_broker_error() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_unsubscribe_bitvmx(&mut bitvmx_broker, 1);
        bitvmx_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::SubscribeBitVMX))
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            bitvmx_broker,
            get_fake_address(),
        );
        let err = monitor.start_bitvmx_monitoring();
        assert!(err.is_err());
        assert!(
            err.as_ref()
                .unwrap_err()
                .to_string()
                .contains("Broker error on SubscribeBitVMX")
        );
    }

    #[test]
    fn test_start_bitvmx_monitoring_fails_if_already_active() {
        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            get_fake_address(),
        );
        monitor.bitvmx_monitoring_active = true;
        let err = monitor.start_bitvmx_monitoring();
        assert!(err.is_err());
    }

    #[test]
    fn test_try_event_returns_some() {
        let log = FakeLogGenerator::new()
            .generate_log("Transfer(address,address,uint256", get_fake_address());

        let event_decoder = EventDecoder::new();

        let expected_event: RskPegManagerEvents = event_decoder.decode(log.clone());

        let mut log_broker = MockBrokerClientApi::new();
        log_broker
            .expect_try_recv()
            .return_once(move || Ok(Some(BrokerResponses::Log(log))));

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            get_fake_address(),
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
            MockBrokerClientApi::new(),
            get_fake_address(),
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
                Ok(Some(BrokerResponses::Block(
                    RskBlockAndUncles::new(block, vec![uncle]).unwrap(),
                )))
            }
        });

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            MockBrokerClientApi::new(),
            get_fake_address(),
        );
        monitor.block_monitoring_active = true;

        let result = monitor.try_block().expect("Failed to receive block");
        assert_eq!(
            result,
            Some(RskBlockAndUncles::new(expected_block_1, vec![expected_uncle_1]).unwrap())
        );
    }

    #[test]
    fn test_try_block_returns_none() {
        let mut block_broker = MockBrokerClientApi::new();
        block_broker.expect_try_recv().return_once(move || Ok(None));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            block_broker,
            MockBrokerClientApi::new(),
            get_fake_address(),
        );
        monitor.block_monitoring_active = true;

        let result = monitor.try_block().expect("Failed to receive block");
        assert_eq!(result, None);
    }

    #[test]
    fn test_try_bitvmx_event_returns_some() {
        let value = BrokerResponses::GetTemporaryPegInAddress(json!("some value"));
        let mock_value = value.clone();
        let mut bitvmx_broker = MockBrokerClientApi::new();
        bitvmx_broker
            .expect_try_recv()
            .return_once(move || Ok(Some(mock_value)));

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            bitvmx_broker,
            get_fake_address(),
        );
        monitor.bitvmx_monitoring_active = true;

        let result = monitor
            .try_bitvmx_event()
            .expect("Failed to receive BitVMX event");
        assert_eq!(result, Some(value));
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
            bitvmx_broker,
            get_fake_address(),
        );
        monitor.bitvmx_monitoring_active = true;

        let result = monitor
            .try_bitvmx_event()
            .expect("Failed to receive BitVMX event");
        assert_eq!(result, None);
    }

    #[test]
    fn test_cancel_event_monitoring_success() {
        let mut log_broker = MockBrokerClientApi::new();
        expect_unsubscribe_logs(&mut log_broker, 1);

        let mut monitor = Monitor::new(
            log_broker,
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            get_fake_address(),
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
            MockBrokerClientApi::new(),
            get_fake_address(),
        );
        monitor.block_monitoring_active = true;

        assert!(monitor.cancel_block_monitoring().is_ok());
        assert!(!monitor.block_monitoring_active);
    }

    #[test]
    fn test_cancel_bitvmx_monitoring_success() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_unsubscribe_bitvmx(&mut bitvmx_broker, 1);

        let mut monitor = Monitor::new(
            MockBrokerClientApi::new(),
            MockBrokerClientApi::new(),
            bitvmx_broker,
            get_fake_address(),
        );
        monitor.bitvmx_monitoring_active = true;

        assert!(monitor.cancel_bitvmx_monitoring().is_ok());
        assert!(!monitor.bitvmx_monitoring_active);
    }

    fn expect_subscribe_logs(log_broker: &mut MockBrokerClientApi, times: usize) {
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::SubscribeLogs(get_fake_address())),
            )
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn expect_subscribe_blocks(block_broker: &mut MockBrokerClientApi, times: usize) {
        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::SubscribeBlocks))
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn expect_subscribe_bitvmx(bitvmx_broker: &mut MockBrokerClientApi, times: usize) {
        bitvmx_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::SubscribeBitVMX))
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn expect_unsubscribe_logs(log_broker: &mut MockBrokerClientApi, times: usize) {
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::UnsubscribeLogs(get_fake_address())),
            )
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn expect_unsubscribe_blocks(block_broker: &mut MockBrokerClientApi, times: usize) {
        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::UnsubscribeBlocks))
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn expect_unsubscribe_bitvmx(bitvmx_broker: &mut MockBrokerClientApi, times: usize) {
        bitvmx_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::UnsubscribeBitVMX))
            .times(times)
            .returning(|_, _| Ok(true));
    }

    fn get_fake_address() -> Address {
        Address::try_from("0x0165878A594ca255338adfa4d48449f69242Eb8F").expect("Invalid address")
    }
}
