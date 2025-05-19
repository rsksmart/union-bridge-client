use crate::types::RskPegManagerEvents::UnknownEvent;
use alloy_primitives::{B256, LogData};
use alloy_sol_types::SolEvent;
use common::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
use common::types::RskLog;
use log::{error, warn};
use union_contracts::bindings::pegmanager::PegManager::RegisteredPegInRequest;

#[derive(Eq, PartialEq, Debug)]
pub enum RskPegManagerEvents {
    RequestAdvanceFunds(RequestAdvanceFunds),
    RemoveRequestAdvanceFunds { peg_out_id: String },
    KickoffAdvanceFunds(KickoffAdvanceFunds),
    RemoveKickoffAdvanceFunds { peg_out_id: String },
    RegisteredPegInRequest(RegisteredPegInRequest),
    UnknownEvent,
}

pub fn decode_rsk_log_to_peg_manager_event(log: RskLog) -> RskPegManagerEvents {
    let parsed_topics: Vec<B256> = log
        .event()
        .topics()
        .iter()
        .filter_map(|topic| topic.parse::<B256>().ok())
        .collect();

    let topic0 = parsed_topics.get(0).cloned();

    let hex_data = match alloy_primitives::hex::decode(&log.event().data()) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to decode RSK log {:?}: {}", log, e);
            return UnknownEvent;
        }
    };

    let log_data = LogData::new(parsed_topics, hex_data.into());
    if log_data.is_none() {
        error!("Failed to create Alloy LogData from rsk_log");
        return UnknownEvent;
    }

    let log_data = log_data.unwrap();

    match topic0 {
        Some(ev) if *ev == RegisteredPegInRequest::SIGNATURE_HASH => {
            decode_register_pegin_event(&log_data)
        }
        Some(ev) if *ev == RequestAdvanceFunds::SIGNATURE_HASH => {
            decode_request_advance_funds_event(&log_data)
        }
        // TODO add other types here in the future
        _ => {
            warn!("Unknown event type in log {:?}", log_data);
            UnknownEvent
        }
    }
}

fn decode_register_pegin_event(log_data: &LogData) -> RskPegManagerEvents {
    match RegisteredPegInRequest::decode_log_data(&log_data, true) {
        Ok(ev) => RskPegManagerEvents::RegisteredPegInRequest(ev),
        Err(_) => UnknownEvent,
    }
}

fn decode_request_advance_funds_event(log_data: &LogData) -> RskPegManagerEvents {
    match RequestAdvanceFunds::decode_log_data(&log_data, true) {
        Ok(ev) => RskPegManagerEvents::RequestAdvanceFunds(ev),
        Err(_) => UnknownEvent,
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct Dispute {
    pub peg_out_id: String,
    req_adv_block: u64,
    req_adv_confirmations: u32,
    kickoff_adv_block: Option<u64>,
    kickoff_adv_confirmations: u32,
}

impl Dispute {
    pub fn new(
        peg_out_id: String,
        req_adv_block: u64,
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

    pub fn set_kickoff(&mut self, block_num: u64) {
        self.kickoff_adv_block = Some(block_num);
    }

    pub fn unset_kickoff(&mut self) {
        self.kickoff_adv_block = None;
    }

    pub fn is_complete_on(&self, last_block: u64) -> bool {
        match self.kickoff_adv_block {
            Some(b) => last_block >= &b + self.kickoff_adv_confirmations as u64,
            None => {
                self.log_delayed_kickoff(last_block);
                false
            }
        }
    }

    fn log_delayed_kickoff(&self, last_block: u64) {
        let diff_blocks = last_block.saturating_sub(
            self.req_adv_block
                .saturating_add(self.req_adv_confirmations as u64),
        );
        if diff_blocks > 0 {
            warn!(
                "KickoffAdvanceFunds not received yet, but we are past the tolerance threshold by {diff_blocks}"
            );
        }
    }
}

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - build on boot via config
pub struct FakePegManagerConfig {}

impl FakePegManagerConfig {
    pub fn get_req_adv_confirmations_for_amount(amount: u64) -> u32 {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-134 - get threshold from config
        if amount < 1000 {
            10
        } else if amount < 10000 {
            20
        } else if amount < 100000 {
            30
        } else {
            40
        }
    }

    pub fn get_kickoff_adv_confirmations_for_amount(amount: u64) -> u32 {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-134 - get threshold from config
        // get threshold from config
        if amount < 1000 {
            5
        } else if amount < 10000 {
            10
        } else if amount < 100000 {
            15
        } else {
            20
        }
    }
}
