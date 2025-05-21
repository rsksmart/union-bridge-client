use crate::event_processor::{DisputedPegOutProcessor, EventProcessor};
use crate::monitor::MonitorApi;
use anyhow::{Context, Result};
use common::constants::coordinator::MONITOR_CHECK_PERIOD;
use common::shutdown_flag::ShutdownFlag;
use log::error;
use std::thread;
use std::time::Duration;

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
            processors: vec![Box::new(DisputedPegOutProcessor::new())],
            check_period: MONITOR_CHECK_PERIOD,
            shutdown_flag,
        }
    }

    pub fn new_for_tests(monitor: M, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
            processors: vec![Box::new(DisputedPegOutProcessor::new())],
            check_period: Duration::from_millis(1),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        self.monitor
            .start_event_monitoring()
            .context("Failed to start event monitoring")?;

        // TODO(Jira) this might be removed once we add resilience in scope of https://rsklabs.atlassian.net/browse/UB-132
        self.monitor
            .cancel_block_monitoring(true)
            .context("Failed to cancel block monitoring")?;

        let result = (|| -> Result<()> {
            loop {
                if !self.is_running() {
                    break;
                }

                let mut message_received = false;

                if let Some(event) = self.monitor.try_event().context("Error getting event")? {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_event(&event) {
                            error!("Error processing event {:?}: {:?}", event, e);
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                // if any processor is waiting for blocks, we need to check for new blocks
                if self.check_processors_waiting_blocks() {
                    self.monitor
                        .start_block_monitoring_if_off()
                        .context("Failed to start block monitoring")?;

                    if let Some(block) = self.monitor.try_block().context("Error getting block")? {
                        self.processors.iter_mut().for_each(|p| {
                            if let Err(e) = p.process_new_block(&block) {
                                error!("Error processing block {:?}: {:?}", block, e);
                            }
                        });

                        // no sleep, try to get new messages asap
                        message_received = true;
                    }
                }

                // if event or block processing made new blocks no longer required, we cancel block monitoring
                if !self.check_processors_waiting_blocks() {
                    self.monitor
                        .cancel_block_monitoring(false)
                        .context("Failed to cancel block monitoring")?;
                }

                if !message_received {
                    thread::sleep(self.check_period);
                }
            }
            Ok(())
        })();

        self.processors.iter().for_each(|p| p.shutdown());

        self.monitor
            .cancel_block_monitoring(false)
            .context("Failed to cancel block monitoring")?;

        self.monitor
            .cancel_event_monitoring()
            .context("Failed to cancel event monitoring")?;

        result
    }

    fn check_processors_waiting_blocks(&mut self) -> bool {
        self.processors.iter().any(|p| p.is_waiting_blocks())
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }
}

#[cfg(test)]
mod tests {
    use crate::coordinator::Coordinator;
    use crate::monitor::MockMonitorApi;
    use crate::types::RskPegManagerEvents;
    use alloy_primitives::U256;
    use common::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
    use common::shutdown_flag::ShutdownFlag;
    use common::test_utils::rsk_block_generator::{
        get_first_default_rsk_block, get_second_default_rsk_block,
    };
    use common::types::RskBlock;
    use std::thread;
    use std::thread::{JoinHandle, sleep};
    use std::time::Duration;

    #[test]
    fn test_coordinator_run_handles_several_events() {
        let mut mock_monitor = MockMonitorApi::new();
        let block_1 = get_first_default_rsk_block();
        let block_2 = get_second_default_rsk_block();

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(
            RequestAdvanceFunds {
                peg_out_id: "peg_out_id_1".to_string(),
                block_hash: block_1.hash().into(),
                amount: 1,
            },
            block_1.number(),
        );

        let event_2: RskPegManagerEvents = RskPegManagerEvents::KickoffAdvanceFunds(
            KickoffAdvanceFunds {
                peg_out_id: "peg_out_id_2".to_string(),
                utxo_id: "utxo_id".to_string(),
                operator_id: "operator_id".to_string(),
                block_hash: block_2.hash().into(),
                required_effort: U256::from(1),
                required_num_blocks: 5,
            },
            block_2.number(),
        );

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
            .expect_cancel_block_monitoring()
            .returning(|_b| Ok(()))
            .times(2);

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(vec![block_1, block_2], &mut mock_monitor);

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

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(
            RequestAdvanceFunds {
                peg_out_id: "peg_out_id".to_string(),
                block_hash: block_1.hash().into(),
                amount: 1,
            },
            block_1.number(),
        );

        let event_2 = RskPegManagerEvents::UnknownEvent;

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
            .expect_cancel_block_monitoring()
            .returning(|_b| Ok(()))
            .times(2);

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
