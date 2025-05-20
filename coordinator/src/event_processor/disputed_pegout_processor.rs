use crate::event_processor::EventProcessor;
use crate::types::{Dispute, FakePegManagerConfig, RskPegManagerEvents, pow_to_effort};
use anyhow::Result;
use common::types::{BlockNumber, RskBlock};
use log::{error, info, warn};
use std::collections::{HashMap, HashSet};

pub struct DisputedPegoutProcessor {
    waiting_blocks: bool,
    disputes: HashMap<String, Dispute>,
    known_blocks: HashSet<RskBlock>,
}

impl DisputedPegoutProcessor {
    pub fn new() -> Self {
        Self {
            waiting_blocks: false,
            disputes: HashMap::new(),
            // TODO(iago) we need to distinguish which ones are canonical and which ones are not
            known_blocks: HashSet::new(),
        }
    }

    fn init_dispute(&mut self, peg_out_id: String, block: BlockNumber, amount: u64) -> Result<()> {
        let dispute = Dispute::new(
            peg_out_id.clone(),
            block,
            FakePegManagerConfig::get_req_effort_for_amount(amount),
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

        self.waiting_blocks = true;

        Ok(())
    }

    fn remove_dispute(&mut self, peg_out_id: &String) -> () {
        let removed_dispute = self.disputes.remove(peg_out_id);
        if removed_dispute.is_none() {
            warn!("Trying to remove unexisting dispute for pegout {peg_out_id}");
        }

        if self.disputes.is_empty() {
            info!("No active disputes, stopping block monitoring");
            self.waiting_blocks = false;
            self.known_blocks.clear();
        }
    }

    fn kickoff_dispute(&mut self, peg_out_id: String, block_num: u64) {
        if let Some(dispute) = self.disputes.get_mut(&peg_out_id) {
            info!("KickoffAdvanceFunds reached for dispute {:?}", dispute);
            dispute.set_kickoff(block_num);
        } else {
            // just log, we don't want to err
            error!("KickoffAdvanceFunds but no dispute found for pegout {peg_out_id}");
        }
    }

    fn undo_kickoff_dispute(&mut self, peg_out_id: String) {
        if let Some(dispute) = self.disputes.get_mut(&peg_out_id) {
            dispute.unset_kickoff();
        } else {
            warn!("RemoveKickoffAdvanceFunds but no dispute found for pegout {peg_out_id}");
        }
    }
}

impl EventProcessor for DisputedPegoutProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(ev) => {
                info!("Handling {:?}...", ev);
                self.init_dispute(
                    ev.peg_out_id.clone().to_string(),
                    BlockNumber::from(ev.block_num),
                    ev.amount,
                )?;
            }
            // TODO(iago) think about how to force removed event
            RskPegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveRequestAdvanceFunds {peg_out_id}...");
                self.remove_dispute(peg_out_id);
            }
            RskPegManagerEvents::KickoffAdvanceFunds(ev) => {
                info!("Handling {:?}...", ev);
                self.kickoff_dispute(ev.peg_out_id.clone(), ev.block_num);
            }
            // TODO(iago) think about how to force removed event
            RskPegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveKickoffAdvanceFunds {peg_out_id}...");
                self.undo_kickoff_dispute(peg_out_id.clone());
            }
            _ => {
                info!("Ignoring {:?}...", event);
                return Ok(()); // ignore unrelated events
            }
        }
        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlock) -> Result<()> {
        let block_pow = pow_to_effort(&block.pow());

        // we want to remove the dispute using the centralized logic (remove_dispute) for cleanup, etc.
        // that's why we have two iterations rather than one using retain or alike
        let mut complete_disputes = Vec::new();
        for (_id, dispute) in self.disputes.iter_mut() {
            dispute.update_pow(block_pow);
            if dispute.has_enough_pow() {
                complete_disputes.push(dispute.clone());
            }
        }

        // in the Union Bridge design, just one withdrawal/dispute will be active at a time, so the loop would not be needed
        // in any case, we leave the code ready for the possibility of multiple withdrawals/disputes in the future
        for dispute in complete_disputes {
            info!("Triggering CheckFork for complete dispute {:?}", dispute);

            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - invoke check fork

            info!("Removing complete dispute {:?}", dispute);
            self.remove_dispute(&dispute.peg_out_id)
        }

        Ok(())
    }

    fn waiting_blocks(&self) -> bool {
        self.waiting_blocks
    }

    fn shutdown(&self) {
        if !self.disputes.is_empty() {
            warn!("{} active disputes found on shutdown!", self.disputes.len());
        }
    }
}
