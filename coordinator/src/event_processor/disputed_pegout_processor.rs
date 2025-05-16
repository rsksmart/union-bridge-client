use crate::event_processor::EventProcessor;
use crate::types::{Dispute, FakePegManagerConfig, PegOutId, RskPegManagerEvents};
use anyhow::{Result, anyhow};
use common::types::{BlockNumber, RskBlock};
use log::{error, info, warn};
use std::collections::{HashMap, HashSet};

pub struct DisputedPegoutProcessor {
    requires_blocks: bool,
    disputes: HashMap<PegOutId, Dispute>,
    known_blocks: HashSet<RskBlock>,
}

impl DisputedPegoutProcessor {
    pub fn new() -> Self {
        Self {
            requires_blocks: false,
            disputes: HashMap::new(),
            known_blocks: HashSet::new(),
        }
    }
    fn init_dispute(
        &mut self,
        peg_out_id: PegOutId,
        block_num: BlockNumber,
        amount: u64,
    ) -> Result<()> {
        let dispute = Dispute::new(
            peg_out_id.clone(),
            block_num,
            FakePegManagerConfig::get_req_adv_confirmations_for_amount(amount),
            FakePegManagerConfig::get_kickoff_adv_confirmations_for_amount(amount),
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

        self.requires_blocks = true;

        Ok(())
    }

    fn remove_dispute(&mut self, peg_out_id: &PegOutId) -> () {
        let removed_dispute = self.disputes.remove(peg_out_id);
        if removed_dispute.is_none() {
            warn!("Trying to remove unexisting dispute for pegout {peg_out_id}");
        }

        if self.disputes.is_empty() {
            info!("No active disputes, stopping block monitoring");
            self.requires_blocks = false;
            self.known_blocks.clear();
        }
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
}

impl EventProcessor for DisputedPegoutProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(ev) => {
                info!("Handling RequestAdvanceFunds...");
                self.init_dispute(ev.peg_out_id.clone(), ev.block_num, ev.amount)?;
            }
            RskPegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                self.remove_dispute(peg_out_id);
            }
            RskPegManagerEvents::KickoffAdvanceFunds {
                peg_out_id,
                block_num,
            } => {
                info!("Received KickoffAdvanceFunds for pegout {peg_out_id}, setting kickoff");
                self.kickoff_dispute(peg_out_id.clone(), *block_num);
            }
            RskPegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id } => {
                info!(
                    "Received RemoveKickoffAdvanceFunds for pegout {peg_out_id}, unsetting kickoff"
                );
                self.undo_kickoff_dispute(peg_out_id.clone());
            }
            _ => return Ok(()), // ignore unrelated events
        }
        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlock) -> Result<()> {
        info!("Received new Block from Block Notifier {:?}", block);

        let block_num = block.number();

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
            self.remove_dispute(&peg_out_id)
        }

        Ok(())
    }

    fn waiting_blocks(&self) -> bool {
        self.requires_blocks
    }

    fn shutdown(&self) {
        if !self.disputes.is_empty() {
            warn!("{} active disputes found on shutdown!", self.disputes.len());
        }
    }
}
