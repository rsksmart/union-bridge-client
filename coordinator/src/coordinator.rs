use crate::{
    event_processor::{DisputedPegoutProcessor, EventProcessor, GetTemporaryPeginAddressProcessor},
    monitor::MonitorApi,
};
use anyhow::{Context, Result};
use common::{constants::coordinator::MONITOR_CHECK_PERIOD, shutdown_flag::ShutdownFlag};
use std::{thread, time::Duration};

pub struct Coordinator<M: MonitorApi> {
    monitor: M,
    processors: Vec<Box<dyn EventProcessor>>,
    check_period: Duration,
    shutdown_flag: ShutdownFlag,
}

impl<M: MonitorApi> Coordinator<M> {
    pub fn new(monitor: M, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
            processors: vec![
                Box::new(DisputedPegoutProcessor::new()),
                Box::new(GetTemporaryPeginAddressProcessor::new()),
            ],
            check_period: MONITOR_CHECK_PERIOD,
            shutdown_flag,
        }
    }

    pub fn new_for_tests(monitor: M, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
            processors: vec![
                Box::new(DisputedPegoutProcessor::new()),
                Box::new(GetTemporaryPeginAddressProcessor::new()),
            ],
            check_period: Duration::from_millis(1),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        self.monitor
            .start_event_monitoring()
            .context("Failed to start event monitoring")?;

        self.monitor
            .start_bitvmx_monitoring()
            .context("Failed to start BitVMX event monitoring")?;

        let result = (|| -> Result<()> {
            loop {
                if !self.is_running() {
                    break;
                }

                if let Some(event) = self
                    .monitor
                    .try_bitvmx_event()
                    .context("Error getting BitVMX event")?
                {
                    // each processor decides if the event is relevant
                    self.processors
                        .iter_mut()
                        .try_for_each(|p| p.process_new_bitvmx_event(&event))?;
                }

                if let Some(event) = self.monitor.try_event().context("Error getting event")? {
                    // each processor decides if the event is relevant
                    self.processors
                        .iter_mut()
                        .try_for_each(|p| p.process_new_event(&event))?;
                }

                // if any processor is waiting for blocks, we need to check for new blocks
                if self.check_processors_waiting_blocks() {
                    self.monitor
                        .start_block_monitoring_if_off()
                        .context("Failed to start block monitoring")?;

                    if let Some(block) = self.monitor.try_block().context("Error getting block")? {
                        self.processors
                            .iter_mut()
                            .try_for_each(|p| p.process_new_block(&block))?;
                    }
                }

                // if event or block processing made new blocks no longer required, we cancel block monitoring
                if !self.check_processors_waiting_blocks() {
                    self.monitor
                        .cancel_block_monitoring_if_on()
                        .context("Failed to cancel block monitoring")?;
                }

                thread::sleep(self.check_period);
            }
            Ok(())
        })();

        self.processors.iter().for_each(|p| p.shutdown());

        self.monitor
            .cancel_bitvmx_monitoring()
            .context("Failed to cancel BitVMX event monitoring")?;

        self.monitor
            .cancel_block_monitoring_if_on()
            .context("Failed to cancel block monitoring")?;

        self.monitor
            .cancel_event_monitoring()
            .context("Failed to cancel event monitoring")?;

        result
    }

    fn check_processors_waiting_blocks(&mut self) -> bool {
        self.processors.iter().any(|p| p.waiting_blocks())
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }
}

#[cfg(test)]
mod tests {
    use crate::{coordinator::Coordinator, monitor::MockMonitorApi, types::RskPegManagerEvents};
    use common::{
        fake_contracts::FakePegManager::RequestAdvanceFunds,
        msg_broker::{
            types::BrokerResponses::GetTemporaryPegInAddress,
        },
        shutdown_flag::ShutdownFlag,
        test_utils::rsk_block_generator::{
            get_first_default_rsk_block, get_second_default_rsk_block,
        },
        types::RskBlock,
    };
    use serde_json::json;
    use std::thread;
    use std::{
        thread::{JoinHandle, sleep},
        time::Duration,
    };

    #[test]
    fn test_coordinator_run_handles_several_events() {
        let mut mock_monitor = MockMonitorApi::new();

        let block_1 = get_first_default_rsk_block();
        let block_2 = get_second_default_rsk_block();

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFunds {
            peg_out_id: "peg_out_id".to_string().parse().unwrap(),
            block_num: block_1.number().value(),
            amount: 1,
        });
        let event_2: RskPegManagerEvents = RskPegManagerEvents::KickoffAdvanceFunds {
            peg_out_id: "peg_out_id".to_string(),
            block_num: block_1.number().value(),
        };

        let bitvmx_event =
            GetTemporaryPegInAddress(json!("GetTemporaryPegInAddress"));

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_bitvmx_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_block_monitoring_if_off()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring_if_on()
            .return_once(|| Ok(()))
            .times(1);

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(vec![block_1, block_2], &mut mock_monitor);

        mock_monitor
            .expect_try_bitvmx_event()
            .returning(move || Ok(Some(bitvmx_event.clone())));

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut coordinator = Coordinator::new_for_tests(mock_monitor, shutdown_flag);
        let result = coordinator.run();

        assert!(result.is_ok());
    }

    #[test]
    fn test_coordinator_run_handles_unknown_event() {
        let mut mock_monitor = MockMonitorApi::new();

        let block_1 = get_first_default_rsk_block();
        let block_2 = get_second_default_rsk_block();

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFunds {
            peg_out_id: "peg_out_id".to_string().parse().unwrap(),
            block_num: block_1.number().value(),
            amount: 1,
        });

        let event_2: RskPegManagerEvents = RskPegManagerEvents::UnknownEvent {};

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_block_monitoring_if_off()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring_if_on()
            .return_once(|| Ok(()))
            .times(1);

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(vec![block_1, block_2], &mut mock_monitor);

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut coordinator = Coordinator::new_for_tests(mock_monitor, shutdown_flag);
        let result = coordinator.run();

        assert!(result.is_ok());
    }

    fn handle_shutdown(shutdown_flag: ShutdownFlag) -> JoinHandle<()> {
        thread::spawn(move || {
            // give time for logic to proceed
            sleep(Duration::from_millis(10));
            shutdown_flag.set();
        })
    }

    fn expect_try_event(client_requests: Vec<RskPegManagerEvents>, monitor: &mut MockMonitorApi) {
        use std::collections::VecDeque;

        monitor.expect_try_event().returning_st({
            let mut responses = client_requests
                .into_iter()
                .map(|e| Ok(Some(e)))
                .collect::<VecDeque<_>>();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn expect_try_block(blocks: Vec<RskBlock>, monitor: &mut MockMonitorApi) {
        use std::collections::VecDeque;

        monitor.expect_try_block().returning_st({
            let mut responses = blocks
                .into_iter()
                .map(|b| Ok(Some(b)))
                .collect::<VecDeque<_>>();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }
}
