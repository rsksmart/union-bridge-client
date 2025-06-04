use crate::event_processor::{EventProcessor, PegOutAdvanceFundsProcessor};
use crate::monitor::MonitorApi;
use anyhow::{Context, Result};
use common::shutdown_flag::ShutdownFlag;
use log::error;
use std::thread;
use std::time::Duration;

const CHECK_PERIOD: Duration = Duration::from_secs(1);

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
            processors: vec![Box::new(PegOutAdvanceFundsProcessor::new())],
            check_period: CHECK_PERIOD,
            shutdown_flag,
        }
    }

    pub fn new_for_tests(monitor: M, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
            processors: vec![Box::new(PegOutAdvanceFundsProcessor::new())],
            check_period: Duration::from_millis(1),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        self.monitor
            .start_event_monitoring()
            .context("Failed to start event monitoring")?;

        self.monitor
            .start_block_monitoring()
            .context("Failed to start block monitoring")?;

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

                if let Some(block) = self.monitor.try_block().context("Error getting block")? {
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_block(&block) {
                            error!("Error processing block {:?}: {:?}", block, e);
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                if !message_received {
                    thread::sleep(self.check_period);
                }
            }
            Ok(())
        })();

        self.processors.iter_mut().for_each(|p| p.shutdown());

        self.monitor
            .cancel_block_monitoring()
            .context("Failed to cancel block monitoring")?;

        self.monitor
            .cancel_event_monitoring()
            .context("Failed to cancel event monitoring")?;

        result
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }
}

#[cfg(test)]
mod tests {
    use crate::coordinator::Coordinator;
    use crate::monitor::MockMonitorApi;
    use crate::types::{KickoffAdvanceFundsEvent, RequestAdvanceFundsEvent, RskPegManagerEvents};
    use alloy_primitives::U256;
    use common::shutdown_flag::ShutdownFlag;
    use common::test_utils::rsk_block_generator::{
        create_block_and_uncles, get_first_default_rsk_block, get_second_default_rsk_block,
    };
    use common::types::RskBlockAndUncles;
    use sc_event_mocking::fake_contracts::FakePegManager::{
        KickoffAdvanceFunds, RequestAdvanceFunds,
    };
    use std::thread;
    use std::thread::{JoinHandle, sleep};
    use std::time::Duration;

    fn create_fake_request_event(peg_out_id: &str) -> RequestAdvanceFunds {
        RequestAdvanceFunds {
            peg_out_id: peg_out_id.to_string(),
            amount: 1000,
        }
    }

    fn create_fake_kickoff_event(peg_out_id: &str) -> KickoffAdvanceFunds {
        KickoffAdvanceFunds {
            peg_out_id: peg_out_id.to_string(),
            utxo_id: "utxo123".to_string(),
            operator_id: "op123".to_string(),
            required_effort: U256::from(1000),
            required_num_blocks: 4,
        }
    }

    #[test]
    fn test_coordinator_run_handles_several_events() {
        let mut mock_monitor = MockMonitorApi::new();
        let (block_1, uncle_1, block_2) = create_block_and_uncles();

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
            inner: create_fake_request_event("peg_out_id_1"),
            block_number: block_1.number(),
            block_hash: block_1.hash().into(),
        });

        let event_2: RskPegManagerEvents =
            RskPegManagerEvents::KickoffAdvanceFunds(KickoffAdvanceFundsEvent {
                inner: create_fake_kickoff_event("peg_out_id_1"),
                block_number: block_2.number(),
                block_hash: block_2.hash().into(),
            });

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_block_monitoring()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring()
            .return_once(|| Ok(()))
            .once();

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(
            vec![
                RskBlockAndUncles::new_no_uncles(block_1),
                RskBlockAndUncles::new(block_2, vec![uncle_1]).unwrap(),
            ],
            &mut mock_monitor,
        );

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

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
            inner: create_fake_request_event("peg_out_id_1"),
            block_number: block_1.number(),
            block_hash: block_1.hash().into(),
        });

        let event_2 = RskPegManagerEvents::UnknownEvent;

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_block_monitoring()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring()
            .return_once(|| Ok(()))
            .once();

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(
            vec![
                RskBlockAndUncles::new_no_uncles(block_1),
                RskBlockAndUncles::new_no_uncles(block_2),
            ],
            &mut mock_monitor,
        );

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut coordinator = Coordinator::new_for_tests(mock_monitor, shutdown_flag);
        let result = coordinator.run();

        assert!(result.is_ok());
    }

    fn handle_shutdown(shutdown_flag: ShutdownFlag) -> JoinHandle<()> {
        thread::spawn(move || {
            // give time for logic to proceed
            sleep(Duration::from_millis(100));
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

    fn expect_try_block(blocks: Vec<RskBlockAndUncles>, monitor: &mut MockMonitorApi) {
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
