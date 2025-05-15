use crate::types::PegManagerEvents;
use anyhow::{Context, Result, bail};
use common::msg_broker::broker::{BROKER_SERVER_ID, BrokerClientApi, BrokerError};
use common::msg_broker::types::{BrokerRequests, BrokerResponses, FakePegManagerConfig};
use common::types::RskBlock;
use log::{debug, info, trace};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait MonitorApi {
    fn start_event_monitoring(&mut self) -> Result<()>;
    fn start_block_monitoring(&mut self) -> Result<()>;
    fn try_event(&mut self) -> Result<Option<PegManagerEvents>>;
    fn try_block(&mut self) -> Result<Option<RskBlock>>;
    fn cancel_event_monitoring(&mut self) -> Result<()>;
    fn cancel_block_monitoring(&mut self) -> Result<()>;
}

pub struct Monitor<BC: BrokerClientApi> {
    block_broker: BC,
    log_broker: BC,
    block_monitoring_active: bool,
    log_monitoring_active: bool,
}

impl<BC: BrokerClientApi> MonitorApi for Monitor<BC> {
    fn start_event_monitoring(&mut self) -> Result<()> {
        self.start_event_monitoring()
    }

    fn start_block_monitoring(&mut self) -> Result<()> {
        self.start_block_monitoring()
    }

    fn try_event(&mut self) -> Result<Option<PegManagerEvents>> {
        self.try_event()
    }

    fn try_block(&mut self) -> Result<Option<RskBlock>> {
        self.try_block()
    }

    fn cancel_event_monitoring(&mut self) -> Result<()> {
        self.cancel_event_monitoring()
    }

    fn cancel_block_monitoring(&mut self) -> Result<()> {
        self.cancel_block_monitoring()
    }
}

impl<T: BrokerClientApi> Monitor<T> {
    pub fn new(block_broker: T, log_broker: T) -> Self {
        Self {
            block_broker,
            log_broker,
            block_monitoring_active: false,
            log_monitoring_active: false,
        }
    }

    // TODO(Jira-CoordinatorResilience) retries, reconnects, etc.

    pub fn start_event_monitoring(&mut self) -> Result<()> {
        if self.log_monitoring_active {
            bail!("Start Log monitoring requested, but it was already active");
        }

        // clean up a potential remaining connection
        self.request_cancel_event_monitoring()
            .context("Cleaning up stalled log connection")?;

        info!("Starting event monitoring");

        let result = self
            .send_to_log_broker(BrokerRequests::SubscribeLogs(
                // TODO(Jira-CheckForkAutomation) forcing PegManager address for every received event for now
                FakePegManagerConfig::get_peg_manager_address(),
            ))
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
            .context("Cleaning up stalled block connection")?;

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

    pub fn try_event(&mut self) -> Result<Option<PegManagerEvents>> {
        if !self.log_monitoring_active {
            bail!("Log monitoring is not active");
        }

        match self.log_broker.try_recv()? {
            Some(BrokerResponses::Log(log)) => {
                info!("Received new Log {:?}", log);
                let event: PegManagerEvents = (&log).into();
                Ok(Some(event))
            }
            Some(e) => {
                bail!("Unexpected response type from Log Notifier {:?}", e)
            }
            None => {
                trace!("No messages from Log Notifier");
                Ok(None)
            }
        }
    }

    pub fn try_block(&mut self) -> Result<Option<RskBlock>> {
        if !self.block_monitoring_active {
            trace!("Block monitoring is not active");
            // no-op
            return Ok(None);
        }

        match self.block_broker.try_recv()? {
            Some(BrokerResponses::Block(b)) => {
                info!("Received new Block {:?}", b);
                Ok(Some(b))
            }
            Some(other) => bail!("Unexpected response type from Block Notifier: {:?}", other),
            None => {
                trace!("No messages from Block Notifier");
                Ok(None)
            }
        }
    }

    pub fn cancel_event_monitoring(&mut self) -> Result<()> {
        if !self.log_monitoring_active {
            debug!("Cancel Log monitoring requested, but it was not active");
            return Ok(());
        }

        if !self.request_cancel_event_monitoring()? {
            bail!("Broker could not deliver UnsubscribeLogs")
        }

        self.log_monitoring_active = false;

        Ok(())
    }

    pub fn cancel_block_monitoring(&mut self) -> Result<()> {
        if !self.block_monitoring_active {
            debug!("Cancel Block monitoring requested, but it was not active");
            return Ok(());
        };

        if !self.request_cancel_block_monitoring()? {
            bail!("Broker could not deliver UnsubscribeBlocks")
        }

        self.block_monitoring_active = false;

        Ok(())
    }

    fn request_cancel_event_monitoring(&mut self) -> Result<bool> {
        self.send_to_log_broker(
            // TODO(Jira-CheckForkAutomation) forcing PegManager address for every received event for now
            BrokerRequests::UnsubscribeLogs(FakePegManagerConfig::get_peg_manager_address()),
        )
        .context("Broker error on UnsubscribeLogs")
    }

    fn request_cancel_block_monitoring(&mut self) -> Result<bool> {
        self.send_to_block_broker(BrokerRequests::UnsubscribeBlocks)
            .context("Broker error on UnsubscribeBlocks")
    }

    fn send_to_block_broker(&mut self, request: BrokerRequests) -> Result<bool, BrokerError> {
        self.block_broker.send(BROKER_SERVER_ID, request)
    }

    fn send_to_log_broker(&mut self, request: BrokerRequests) -> Result<bool, BrokerError> {
        self.log_broker.send(BROKER_SERVER_ID, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use common::msg_broker::broker::{BROKER_SERVER_ID, MockBrokerClientApi};
    use common::msg_broker::types::{BrokerRequests, FakePegManagerConfig};
    use common::test_utils::rsk_block_generator::get_first_default_rsk_block;
    use common::test_utils::rsk_log_generator::FakeLogGenerator;
    use common::test_utils::rsk_utils::generate_fake_address;
    use mockall::predicate::*;

    #[test]
    fn test_start_event_monitoring_success() {
        let mut log_broker = MockBrokerClientApi::new();
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::UnsubscribeLogs(
                    FakePegManagerConfig::get_peg_manager_address(),
                )),
            )
            .return_once(|_, _| Ok(true));

        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::SubscribeLogs(
                    FakePegManagerConfig::get_peg_manager_address(),
                )),
            )
            .return_once(|_, _| Ok(true));

        let mut monitor = Monitor::new(MockBrokerClientApi::new(), log_broker);
        assert!(monitor.start_event_monitoring().is_ok());
        assert!(monitor.log_monitoring_active);
    }

    #[test]
    fn test_start_event_monitoring_fails_on_broker_error() {
        let mut log_broker = MockBrokerClientApi::new();
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::UnsubscribeLogs(
                    FakePegManagerConfig::get_peg_manager_address(),
                )),
            )
            .return_once(|_, _| Ok(true));

        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::SubscribeLogs(
                    FakePegManagerConfig::get_peg_manager_address(),
                )),
            )
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(MockBrokerClientApi::new(), log_broker);
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
        let mut monitor = Monitor::new(MockBrokerClientApi::new(), MockBrokerClientApi::new());
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
        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::UnsubscribeBlocks))
            .return_once(|_, _| Ok(true));

        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::SubscribeBlocks))
            .return_once(|_, _| Ok(true));

        let mut monitor = Monitor::new(block_broker, MockBrokerClientApi::new());
        assert!(monitor.start_block_monitoring().is_ok());
        assert!(monitor.block_monitoring_active);
    }

    #[test]
    fn test_start_block_monitoring_fails_on_broker_error() {
        let mut block_broker = MockBrokerClientApi::new();
        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::UnsubscribeBlocks))
            .return_once(|_, _| Ok(true));

        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::SubscribeBlocks))
            .return_once(|_, _| Err(BrokerError::UnknownError(anyhow!("fake error"))));

        let mut monitor = Monitor::new(block_broker, MockBrokerClientApi::new());
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
        let mut monitor = Monitor::new(MockBrokerClientApi::new(), MockBrokerClientApi::new());
        monitor.block_monitoring_active = true;
        let err = monitor.start_block_monitoring();
        assert!(err.is_err());
        assert!(
            err.as_ref()
                .unwrap_err()
                .to_string()
                .contains("already active")
        );
    }

    #[test]
    fn test_try_event_returns_some() {
        let log = FakeLogGenerator::new()
            .generate_log("Transfer(address,address,uint256", generate_fake_address(1));
        let expected_event: PegManagerEvents = (&log).try_into().unwrap(); // or use your From impl

        let mut log_broker = MockBrokerClientApi::new();
        log_broker
            .expect_try_recv()
            .return_once(move || Ok(Some(BrokerResponses::Log(log))));

        let mut monitor = Monitor::new(MockBrokerClientApi::new(), log_broker);
        monitor.log_monitoring_active = true;

        let result = monitor.try_event().expect("Failed to receive event");
        assert_eq!(result, Some(expected_event));
    }

    #[test]
    fn test_try_event_returns_none() {
        let mut log_broker = MockBrokerClientApi::new();
        log_broker.expect_try_recv().return_once(move || Ok(None));

        let mut monitor = Monitor::new(MockBrokerClientApi::new(), log_broker);
        monitor.log_monitoring_active = true;

        let result = monitor.try_event().expect("Failed to receive event");
        assert_eq!(result, None);
    }

    #[test]
    fn test_try_block_returns_some() {
        let block = get_first_default_rsk_block();
        let mut block_broker = MockBrokerClientApi::new();
        block_broker.expect_try_recv().return_once({
            let block = block.clone();
            move || Ok(Some(BrokerResponses::Block(block.clone())))
        });

        let mut monitor = Monitor::new(block_broker, MockBrokerClientApi::new());
        monitor.block_monitoring_active = true;

        let result = monitor.try_block().expect("Failed to receive block");
        assert_eq!(result, Some(block));
    }

    #[test]
    fn test_try_block_returns_none() {
        let mut block_broker = MockBrokerClientApi::new();
        block_broker.expect_try_recv().return_once(move || Ok(None));

        let mut monitor = Monitor::new(block_broker, MockBrokerClientApi::new());
        monitor.block_monitoring_active = true;

        let result = monitor.try_block().expect("Failed to receive block");
        assert_eq!(result, None);
    }

    #[test]
    fn test_cancel_event_monitoring_success() {
        let mut log_broker = MockBrokerClientApi::new();
        log_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                eq(BrokerRequests::UnsubscribeLogs(
                    FakePegManagerConfig::get_peg_manager_address(),
                )),
            )
            .return_once(|_, _| Ok(true));

        let mut monitor = Monitor::new(MockBrokerClientApi::new(), log_broker);
        monitor.log_monitoring_active = true;

        assert!(monitor.cancel_event_monitoring().is_ok());
        assert!(!monitor.log_monitoring_active);
    }

    #[test]
    fn test_cancel_block_monitoring_success() {
        let mut block_broker = MockBrokerClientApi::new();
        block_broker
            .expect_send()
            .with(eq(BROKER_SERVER_ID), eq(BrokerRequests::UnsubscribeBlocks))
            .return_once(|_, _| Ok(true));

        let mut monitor = Monitor::new(block_broker, MockBrokerClientApi::new());
        monitor.block_monitoring_active = true;

        assert!(monitor.cancel_block_monitoring().is_ok());
        assert!(!monitor.block_monitoring_active);
    }
}
