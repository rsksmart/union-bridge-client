use crate::monitor::MonitorApi;
use crate::types::{Dispute, PegManagerEvents, PegOutId};
use anyhow::{Context, Result, anyhow};
use common::constants::coordinator::MONITOR_CHECK_PERIOD;
use common::msg_broker::types::FakePegManagerConfig;
use common::shutdown_flag::ShutdownFlag;
use common::types::{BlockNumber, RskBlock};
use log::{error, info, warn};
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;

const FAKE_AMOUNT: u64 = 1000; // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - create one fake event of each check fork event type

pub struct Coordinator<M: MonitorApi> {
    monitor: M,
    check_period: Duration,
    disputes: HashMap<PegOutId, Dispute>,
    known_blocks: HashSet<RskBlock>,
    shutdown_flag: ShutdownFlag,
}

impl<M: MonitorApi> Coordinator<M> {
    pub fn new(monitor: M, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
            check_period: MONITOR_CHECK_PERIOD,
            disputes: HashMap::new(),
            known_blocks: HashSet::new(),
            shutdown_flag,
        }
    }

    pub fn new_for_tests(monitor: M, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
            check_period: Duration::from_millis(1),
            disputes: HashMap::new(),
            known_blocks: HashSet::new(),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        self.monitor
            .start_event_monitoring()
            .context("Failed to start event monitoring")?;

        let result = (|| -> Result<()> {
            loop {
                if !self.is_running() {
                    break;
                }

                if let Some(event) = self.monitor.try_event().context("Error getting event")? {
                    self.process_event(event)
                        .context("Error processing event")?;
                }

                if let Some(block) = self.monitor.try_block().context("Error getting block")? {
                    self.process_new_block(block)
                        .context("Error processing block")?;
                }

                thread::sleep(self.check_period);
            }
            Ok(())
        })();

        if !self.disputes.is_empty() {
            warn!("{} active disputes found on shutdown!", self.disputes.len());
        }

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

    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - This piece will be refactored with a factory pattern or a similar approach and properly tested
    fn process_event(&mut self, event: PegManagerEvents) -> Result<()> {
        match event {
            PegManagerEvents::RequestAdvanceFunds {
                peg_out_id,
                block_num,
            } => {
                info!("Received RequestAdvanceFunds for pegout {peg_out_id}, initialising dispute");
                self.init_dispute(peg_out_id, block_num)
                    .context("Initializing dispute")?;
            }
            PegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                info!("Received RemoveReqAdvFunds for pegout {peg_out_id}, removing dispute");
                self.remove_dispute(&peg_out_id)
                    .context("Removing dispute")?;
            }
            PegManagerEvents::KickoffAdvanceFunds {
                peg_out_id,
                block_num,
            } => {
                info!("Received KickoffAdvanceFunds for pegout {peg_out_id}, setting kickoff");
                self.kickoff_dispute(peg_out_id, block_num);
            }
            PegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id } => {
                info!(
                    "Received RemoveKickoffAdvanceFunds for pegout {peg_out_id}, unsetting kickoff"
                );
                self.undo_kickoff_dispute(peg_out_id);
            }
            PegManagerEvents::UnknownEvent {} => {
                // just log, we don't want to err
                error!("Unknown event");
            }
        }
        Ok(())
    }

    fn undo_kickoff_dispute(&mut self, peg_out_id: PegOutId) {
        if let Some(dispute) = self.disputes.get_mut(&peg_out_id) {
            dispute.unset_kickoff();
        } else {
            warn!("RemoveKickoffAdvanceFunds but no dispute found for pegout {peg_out_id}");
        }
    }

    fn kickoff_dispute(&mut self, peg_out_id: PegOutId, block_num: BlockNumber) {
        if let Some(dispute) = self.disputes.get_mut(&peg_out_id) {
            info!("KickoffAdvanceFunds reached for dispute {:?}", dispute);
            dispute.set_kickoff(block_num);
        } else {
            // just log, we don't want to err
            error!("KickoffAdvanceFunds but no dispute found for pegout {peg_out_id}");
        }
    }

    fn init_dispute(&mut self, peg_out_id: PegOutId, block_num: BlockNumber) -> Result<()> {
        let dispute = Dispute::new(
            peg_out_id.clone(),
            block_num,
            FakePegManagerConfig::get_req_adv_confirmations_for_amount(FAKE_AMOUNT),
            FakePegManagerConfig::get_kickoff_adv_confirmations_for_amount(FAKE_AMOUNT),
        );

        if self.disputes.contains_key(&dispute.peg_out_id) {
            error!("Dispute {:?} already exists", dispute);
            // we don't want to err, so we just skip this event
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            return Ok(());
        }

        if self.disputes.len() == 1 {
            // we don't want to err, so we just skip this event
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            error!("More than one dispute detected, this is not expected on Union Bridge Design");
            return Ok(());
        }

        info!("Init dispute {dispute:?}, count {}", self.disputes.len());
        self.disputes.insert(peg_out_id, dispute);

        self.monitor
            .start_block_monitoring()
            .context("Failed to start block monitoring")?;

        Ok(())
    }

    fn remove_dispute(&mut self, peg_out_id: &PegOutId) -> Result<()> {
        let removed_dispute = self.disputes.remove(peg_out_id);
        if removed_dispute.is_none() {
            warn!("Trying to remove unexisting dispute for pegout {peg_out_id}");
        }

        if self.disputes.is_empty() {
            info!("No active disputes, stopping block monitoring");
            self.monitor.cancel_block_monitoring()?;
            self.known_blocks.clear();
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: RskBlock) -> Result<()> {
        info!("Received new Block from Block Notifier {:?}", block);

        let block_num = block.number();

        self.known_blocks.insert(block);

        // we want to remove the dispute using the centralized logic (remove_dispute) for cleanup, etc.
        // that's why we have two iterations rather than one using retain or alike

        let complete_disputes: Vec<PegOutId> = self
            .disputes
            .iter()
            .filter(|(_, d)| d.is_complete_on(&block_num))
            .map(|(id, _)| id.clone())
            .collect();

        // in the Union Bridge design, just one withdrawal/dispute will be active at a time, so the loop would not be needed
        // in any case, we leave the code ready for the possibility of multiple withdrawals/disputes in the future
        for peg_out_id in complete_disputes {
            let dispute = self
                .disputes
                .get(&peg_out_id)
                .ok_or(anyhow!("Complete dispute not found"))?;

            info!("Triggering CheckFork for complete dispute {:?}", dispute);

            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - invoke check fork

            info!("Removing complete dispute {:?}", dispute);
            self.remove_dispute(&peg_out_id)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::coordinator::Coordinator;
    use crate::monitor::MockMonitorApi;
    use crate::types::PegManagerEvents;
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

        let event_1: PegManagerEvents = PegManagerEvents::RequestAdvanceFunds {
            peg_out_id: "peg_out_id".to_string(),
            block_num: block_1.number(),
        };

        let event_2: PegManagerEvents = PegManagerEvents::KickoffAdvanceFunds {
            peg_out_id: "peg_out_id".to_string(),
            block_num: block_1.number(),
        };

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_start_block_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring()
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

    #[test]
    fn test_coordinator_run_handles_unknown_event() {
        let mut mock_monitor = MockMonitorApi::new();

        let block_1 = get_first_default_rsk_block();
        let block_2 = get_second_default_rsk_block();

        let event_1: PegManagerEvents = PegManagerEvents::RequestAdvanceFunds {
            peg_out_id: "peg_out_id".to_string(),
            block_num: block_1.number(),
        };

        let event_2: PegManagerEvents = PegManagerEvents::UnknownEvent {};

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_start_block_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring()
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

    fn expect_try_event(client_requests: Vec<PegManagerEvents>, monitor: &mut MockMonitorApi) {
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
