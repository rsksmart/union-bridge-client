use crate::types::RskPegManagerEvents::UnknownEvent;
use alloy_primitives::{B256, LogData};
use alloy_sol_types::SolEvent;
use common::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
use common::types::{BlockNumber, BlockPow, RskLog};
use log::{error, info, warn};
use primitive_types::U256;
use std::collections::HashMap;
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

type DecoderFn = fn(&LogData) -> RskPegManagerEvents;

pub struct EventDecoder {
    dispatch: HashMap<B256, DecoderFn>,
}

impl EventDecoder {
    pub fn new() -> Self {
        let mut dispatcher = HashMap::new();
        dispatcher.insert(
            RegisteredPegInRequest::SIGNATURE_HASH,
            Self::decode_register_pegin_event as DecoderFn,
        );
        dispatcher.insert(
            RequestAdvanceFunds::SIGNATURE_HASH,
            Self::decode_request_advance_funds_event as DecoderFn,
        );
        dispatcher.insert(
            KickoffAdvanceFunds::SIGNATURE_HASH,
            Self::decode_kickoff_advance_funds_event as DecoderFn,
        );
        Self {
            dispatch: dispatcher,
        }
    }

    pub fn decode(&self, log: &RskLog) -> RskPegManagerEvents {
        let (topic0, log_data) = match Self::parse_rsk_log_to_alloy(log) {
            Some(value) => value,
            None => return UnknownEvent,
        };

        match self.dispatch.get(&topic0) {
            Some(decoder_fn) => decoder_fn(&log_data),
            None => {
                warn!("Unknown event type for log: {:?}", log);
                RskPegManagerEvents::UnknownEvent
            }
        }
    }

    fn parse_rsk_log_to_alloy(log: &RskLog) -> Option<(B256, LogData)> {
        let parsed_topics: Vec<B256> = log
            .event()
            .topics()
            .iter()
            .filter_map(|topic| topic.parse::<B256>().ok())
            .collect();

        let hex_data = match alloy_primitives::hex::decode(&log.event().data()) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to decode RSK log {:?}: {}", log, e);
                return None;
            }
        };

        let log_data = match LogData::new(parsed_topics, hex_data.into()) {
            Some(data) => data,
            None => {
                error!("Failed to create Alloy LogData from rsk_log");
                return None;
            }
        };

        let topic0 = match log_data.topics().first() {
            Some(topic) => *topic,
            None => {
                warn!("No topics found in log: {:?}", log);
                return None;
            }
        };

        Some((topic0, log_data))
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

    fn decode_kickoff_advance_funds_event(log_data: &LogData) -> RskPegManagerEvents {
        match KickoffAdvanceFunds::decode_log_data(&log_data, true) {
            Ok(ev) => RskPegManagerEvents::KickoffAdvanceFunds(ev),
            Err(_) => UnknownEvent,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Dispute {
    pub peg_out_id: String,
    req_adv_block: BlockNumber,
    kickoff_adv_block: Option<u64>,
    kickoff_pending_effort: U256,
}

impl Dispute {
    pub fn new(peg_out_id: String, req_adv_block: BlockNumber, kickoff_req_effort: U256) -> Self {
        Self {
            peg_out_id,
            req_adv_block,
            kickoff_adv_block: None,
            kickoff_pending_effort: kickoff_req_effort,
        }
    }

    pub fn set_kickoff(&mut self, block_num: u64) {
        self.kickoff_adv_block = Some(block_num);
    }

    pub fn unset_kickoff(&mut self) {
        self.kickoff_adv_block = None;
    }

    pub fn update_pow(&mut self, block_effort: U256) -> () {
        if let Some(_b) = self.kickoff_adv_block {
            self.kickoff_pending_effort = self.kickoff_pending_effort.saturating_sub(block_effort);
            info!(
                "Dispute {}: reduced kickoff_proved_pending_effort by {} to {})",
                self.peg_out_id, block_effort, self.kickoff_pending_effort
            );
        }
    }

    pub fn has_enough_pow(&self) -> bool {
        self.kickoff_pending_effort.is_zero()
    }
}

// TODO(iago) calculate reasonable values and build on boot via config
pub struct FakePegManagerConfig {}

impl FakePegManagerConfig {
    pub fn get_req_effort_for_amount(amount: u64) -> U256 {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-134 - get threshold from config
        if amount < 1000 {
            U256::from_dec_str("1000000000000").expect("Failed to parse U256")
        } else if amount < 10000 {
            U256::from_dec_str("2000000000000").expect("Failed to parse U256")
        } else if amount < 100000 {
            U256::from_dec_str("3000000000000").expect("Failed to parse U256")
        } else {
            U256::from_dec_str("4000000000000").expect("Failed to parse U256")
        }
    }
}

#[cfg(not(feature = "anvil"))]
pub fn pow_to_effort(pow: &BlockPow) -> U256 {
    let pow_dec: U256 = U256::from_big_endian(pow.value().as_bytes());
    U256::MAX.checked_div(pow_dec).unwrap_or_else(|| {
        error!("Received 0 as pow");
        U256::zero()
    })
}

#[cfg(feature = "anvil")]
pub fn pow_to_effort(_pow: &BlockPow) -> U256 {
    U256::from(250000000000u64)
}
