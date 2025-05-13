use common::msg_broker::types::FakePegManagerConfig;
use common::types::{BlockNumber, RskLog, Selector};
use log::warn;
use std::ops::Add;

pub type PegOutId = String;

pub enum PegManagerEvents {
    RequestAdvanceFunds {
        // TODO add other fields
        peg_out_id: PegOutId,
        block_num: BlockNumber,
    },
    RemoveRequestAdvanceFunds {
        peg_out_id: PegOutId,
    },
    KickoffAdvanceFunds {
        // TODO add other fields
        peg_out_id: PegOutId,
        block_num: BlockNumber,
    },
    RemoveKickoffAdvanceFunds {
        peg_out_id: PegOutId,
    },
    UnknownEvent {
        peg_out_id: PegOutId,
    },
}

impl From<&RskLog> for PegManagerEvents {
    fn from(log: &RskLog) -> Self {
        let _selector: Selector = log.into();

        // TODO(Jira-PegManagerInRootstock)
        let selector = FakePegManagerConfig::get_request_advance_funds_selector();

        let peg_out_id = Self::get_peg_out_id_from_log(&log);
        let block_num = log.info().block_number();

        if selector == FakePegManagerConfig::get_request_advance_funds_selector() {
            if log.info().removed() {
                PegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id }
            } else {
                PegManagerEvents::RequestAdvanceFunds {
                    peg_out_id,
                    block_num,
                }
            }
        } else if selector == FakePegManagerConfig::get_kickoff_advance_funds_selector() {
            if log.info().removed() {
                PegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id }
            } else {
                PegManagerEvents::KickoffAdvanceFunds {
                    peg_out_id,
                    block_num,
                }
            }
        } else {
            PegManagerEvents::UnknownEvent { peg_out_id }
        }
    }
}

impl PegManagerEvents {
    fn get_peg_out_id_from_log(log: &RskLog) -> PegOutId {
        // TODO(Jira-PegManagerInRootstock) replace with actual info from event
        format!(
            "fake_pegout_id_{}_{}_{}",
            log.info().block_hash(),
            log.info().tx_hash(),
            log.info().log_index()
        )
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct Dispute {
    pub peg_out_id: PegOutId,
    req_adv_block: BlockNumber,
    req_adv_confirmations: u32,
    kickoff_adv_block: Option<BlockNumber>,
    kickoff_adv_confirmations: u32,
}

const REQ_ADV_CONFIRMATIONS_TOLERANCE_THRESHOLD: f64 = 1.10;

impl Dispute {
    pub fn new(
        peg_out_id: PegOutId,
        req_adv_block: BlockNumber,
        req_adv_confirmations: u32,
        kickoff_adv_confirmations: u32,
    ) -> Self {
        Self {
            peg_out_id,
            req_adv_block,
            req_adv_confirmations,
            kickoff_adv_block: None,
            kickoff_adv_confirmations,
        }
    }

    pub fn set_kickoff(&mut self, block_num: BlockNumber) {
        self.kickoff_adv_block = Some(block_num);
    }

    pub fn unset_kickoff(&mut self) {
        self.kickoff_adv_block = None;
    }

    pub fn is_complete_on(&self, last_block: &BlockNumber) -> bool {
        match self.kickoff_adv_block {
            Some(b) => last_block >= &b.add(self.kickoff_adv_confirmations as u64),
            None => {
                self.log_delayed_kickoff(last_block);
                false
            }
        }
    }

    fn log_delayed_kickoff(&self, last_block: &BlockNumber) {
        let tolerance =
            self.req_adv_confirmations as f64 * REQ_ADV_CONFIRMATIONS_TOLERANCE_THRESHOLD;
        if last_block <= &self.req_adv_block.add(tolerance as u64) {
            warn!("KickoffAdvanceFunds not received yet, but we are past the tolerance threshold");
        }
    }
}
