use crate::monitor::Monitor;
use crate::types::{Dispute, PegManagerEvents, PegOutId};
use anyhow::{Context, Result, anyhow};
use common::constants::coordinator::MONITOR_CHECK_PERIOD;
use common::msg_broker::broker::BrokerClientApi;
use common::msg_broker::types::FakePegManagerConfig;
use common::shutdown_flag::ShutdownFlag;
use common::types::{BlockNumber, RskBlock};
use log::{error, info, warn};
use std::collections::{HashMap, HashSet};
use std::thread;

const FAKE_AMOUNT: u64 = 1000; // TODO(Jira-PegManagerInRootstock) replace with actual amount

pub struct Coordinator<T: BrokerClientApi> {
    monitor: Monitor<T>,
    disputes: HashMap<PegOutId, Dispute>,
    known_blocks: HashSet<RskBlock>,
    shutdown_flag: ShutdownFlag,
}

impl<T: BrokerClientApi> Coordinator<T> {
    pub fn new(monitor: Monitor<T>, shutdown_flag: ShutdownFlag) -> Self {
        Self {
            monitor,
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

                thread::sleep(MONITOR_CHECK_PERIOD);
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
            PegManagerEvents::UnknownEvent { peg_out_id } => {
                // just log, we don't want to err
                error!("Unknown event for peg_out: {}", peg_out_id);
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
