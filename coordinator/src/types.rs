use crate::types::RskPegManagerEvents::UnknownEvent;
use alloy_primitives::{B256, LogData};
use alloy_sol_types::SolEvent;
use common::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
use common::types::{BlockNumber, RskLog};
use log::{error, warn};
use std::collections::HashMap;
use union_contracts::bindings::pegmanager::PegManager::RegisteredPegInRequest;

#[derive(Eq, PartialEq, Debug)]
pub enum RskPegManagerEvents {
    RequestAdvanceFunds(RequestAdvanceFunds, BlockNumber),
    RemoveRequestAdvanceFunds { peg_out_id: String },
    KickoffAdvanceFunds(KickoffAdvanceFunds, BlockNumber),
    RemoveKickoffAdvanceFunds { peg_out_id: String },
    RegisteredPegInRequest(RegisteredPegInRequest, BlockNumber),
    UnknownEvent,
}

type DecoderFn = fn(&LogData, BlockNumber) -> RskPegManagerEvents;

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

    pub fn decode(&self, log: RskLog) -> RskPegManagerEvents {
        let (topic0, log_data) = match Self::parse_rsk_log_to_alloy(&log) {
            Some(value) => value,
            None => return UnknownEvent,
        };

        match self.dispatch.get(&topic0) {
            Some(decoder_fn) => decoder_fn(&log_data, log.info().block_number()),
            None => {
                warn!("Unknown event type for log: {:?}", log);
                UnknownEvent
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

    fn decode_register_pegin_event(
        log_data: &LogData,
        block_num: BlockNumber,
    ) -> RskPegManagerEvents {
        match RegisteredPegInRequest::decode_log_data(&log_data, true) {
            Ok(ev) => RskPegManagerEvents::RegisteredPegInRequest(ev, block_num),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_request_advance_funds_event(
        log_data: &LogData,
        block_num: BlockNumber,
    ) -> RskPegManagerEvents {
        match RequestAdvanceFunds::decode_log_data(&log_data, true) {
            Ok(ev) => RskPegManagerEvents::RequestAdvanceFunds(ev, block_num),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_kickoff_advance_funds_event(
        log_data: &LogData,
        block_num: BlockNumber,
    ) -> RskPegManagerEvents {
        match KickoffAdvanceFunds::decode_log_data(&log_data, true) {
            Ok(ev) => RskPegManagerEvents::KickoffAdvanceFunds(ev, block_num),
            Err(_) => UnknownEvent,
        }
    }
}
