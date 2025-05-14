use crate::types::PegManagerEvents;
use anyhow::{Context, Result, bail};
use common::msg_broker::broker::{BROKER_SERVER_ID, BrokerClient, BrokerClientApi, BrokerError};
use common::msg_broker::types::{BrokerRequests, BrokerResponses, FakePegManagerConfig};
use common::types::RskBlock;
use log::{debug, info, trace};

pub struct Monitor {
    block_broker: BrokerClient,
    log_broker: BrokerClient,
    block_monitoring_active: bool,
    log_monitoring_active: bool,
}
impl Monitor {
    pub fn new(block_broker: BrokerClient, log_broker: BrokerClient) -> Self {
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
                // TODO(Jira-PegManagerInRootstock) forcing PegManager address for every received event for now
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

    fn request_cancel_event_monitoring(&mut self) -> Result<bool> {
        self.send_to_log_broker(
            // TODO(Jira-PegManagerInRootstock) forcing PegManager address for every received event for now
            BrokerRequests::UnsubscribeLogs(FakePegManagerConfig::get_peg_manager_address()),
        )
        .context("Broker error on UnsubscribeLogs")
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
