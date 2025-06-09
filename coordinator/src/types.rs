use crate::types::RskPegManagerEvents::UnknownEvent;
use alloy_primitives::{B256, LogData};
use alloy_sol_types::SolEvent;
use common::types::{BlockHash, BlockNumber, RskLog};
use log::{error, warn};
use sc_event_mocking::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
use std::collections::HashMap;
use union_contracts::bindings::pegmanager::PegManager::RegisteredPegInRequest;

#[derive(Eq, PartialEq, Debug)]
pub enum RskPegManagerEvents {
    RequestAdvanceFunds(RequestAdvanceFundsEvent), // temporarily mock, no need to test it
    RemoveRequestAdvanceFunds { peg_out_id: String }, // temporarily mock, no need to test it
    KickoffAdvanceFunds(KickoffAdvanceFundsEvent), // temporarily mock, no need to test it
    RemoveKickoffAdvanceFunds { peg_out_id: String }, // temporarily mock, no need to test it
    RegisteredPegInRequest(RegisteredPegInRequestEvent),
    RemoveRegisteredPegInRequest(RegisteredPegInRequestEvent),
    UnknownEvent,
}

pub type RequestAdvanceFundsEvent = EventWithBlock<RequestAdvanceFunds>;
pub type KickoffAdvanceFundsEvent = EventWithBlock<KickoffAdvanceFunds>;
pub type RegisteredPegInRequestEvent = EventWithBlock<RegisteredPegInRequest>;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct EventWithBlock<T> {
    pub inner: T,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

pub type EventStatus = bool;

type DecoderFn = fn(&LogData, BlockNumber, BlockHash, EventStatus) -> RskPegManagerEvents;
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

        let block_num = log.info().block_number();
        let block_hash = log.info().block_hash();
        match self.dispatch.get(&topic0) {
            Some(decoder_fn) => decoder_fn(&log_data, block_num, block_hash, log.info().removed()),
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
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
    ) -> RskPegManagerEvents {
        match RegisteredPegInRequest::decode_log_data(&log_data, true) {
            Ok(ev) if !removed => {
                RskPegManagerEvents::RegisteredPegInRequest(RegisteredPegInRequestEvent {
                    inner: ev,
                    block_number,
                    block_hash,
                })
            }
            Ok(ev) => {
                RskPegManagerEvents::RemoveRegisteredPegInRequest(RegisteredPegInRequestEvent {
                    inner: ev,
                    block_number,
                    block_hash,
                })
            }
            Err(_) => UnknownEvent,
        }
    }

    fn decode_request_advance_funds_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
    ) -> RskPegManagerEvents {
        match RequestAdvanceFunds::decode_log_data(&log_data, true) {
            Ok(event) if !removed => {
                RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
                    inner: event,
                    block_number,
                    block_hash,
                })
            }
            Ok(event) => RskPegManagerEvents::RemoveRequestAdvanceFunds {
                peg_out_id: event.peg_out_id,
            },
            Err(_) => UnknownEvent,
        }
    }

    fn decode_kickoff_advance_funds_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
    ) -> RskPegManagerEvents {
        match KickoffAdvanceFunds::decode_log_data(&log_data, true) {
            Ok(event) if !removed => {
                RskPegManagerEvents::KickoffAdvanceFunds(KickoffAdvanceFundsEvent {
                    inner: event,
                    block_number,
                    block_hash,
                })
            }
            Ok(event) => RskPegManagerEvents::RemoveKickoffAdvanceFunds {
                peg_out_id: event.peg_out_id,
            },
            Err(_) => UnknownEvent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use common::test_utils::rsk_log_generator::{FakeLogGenerator, event_signature_to_topic};
    use common::test_utils::rsk_utils::generate_fake_address;
    use common::types::{BlockHash, LogEvent, LogInfo, RskLog};
    use primitive_types::H256;
    #[test]
    fn test_decode_unknown_event() {
        let decoder = EventDecoder::new();
        let log = FakeLogGenerator::new().generate_log(
            "Transfer(address,address,uint256)",
            generate_fake_address(1),
        );

        let result = decoder.decode(log);
        assert_eq!(result, UnknownEvent);
    }

    #[test]
    fn test_decode_invalid_data() {
        let log_event: LogEvent = LogEvent::new(
            "fake".to_string(),
            vec![event_signature_to_topic(
                "Transfer(address,address,uint256)",
            )],
        );

        let log_info = LogInfo::new(
            generate_fake_address(1),
            BlockHash::from(H256::random()),
            1.into(),
            H256::random().to_string(),
            1,
            false,
        );

        let log = RskLog::new(log_info, log_event);

        let decoder = EventDecoder::new();
        let result = decoder.decode(log);
        assert_eq!(result, UnknownEvent);
    }

    #[test]
    fn test_decode_no_topics() {
        let log_event: LogEvent = LogEvent::new(
            "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            vec![],
        );

        let log_info = LogInfo::new(
            generate_fake_address(1),
            BlockHash::from(H256::random()),
            1.into(),
            H256::random().to_string(),
            1,
            false,
        );

        let log = RskLog::new(log_info, log_event);

        let decoder = EventDecoder::new();
        let result = decoder.decode(log);
        assert_eq!(result, UnknownEvent);
    }

    #[test]
    fn test_decode_invalid_topics() {
        let topic = event_signature_to_topic("Transfer(address,address,uint256)");
        let log_event: LogEvent = LogEvent::new(
            "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            vec![
                topic.clone(),
                topic.clone(),
                topic.clone(),
                topic.clone(),
                topic,
            ], // 5 topics, invalid
        );

        let log_info = LogInfo::new(
            generate_fake_address(1),
            BlockHash::from(H256::random()),
            1.into(),
            H256::random().to_string(),
            1,
            false,
        );

        let log = RskLog::new(log_info, log_event);

        let decoder = EventDecoder::new();
        let result = decoder.decode(log);
        assert_eq!(result, UnknownEvent);
    }

    #[test]
    fn test_decode_request_pegin_event() {
        let expected_block_hash = H256::from_low_u64_be(123);
        let expected_block_num = 789;

        let expected_event = RegisteredPegInRequest {
            blockHash: expected_block_hash
                .as_bytes()
                .try_into()
                .expect("Failed to decode block hash"),
            txHash: H256::from_low_u64_be(456)
                .as_bytes()
                .try_into()
                .expect("Failed to decode tx hash"),
            vout: 1,
            value: 1000,
            packetNumber: U256::from(33),
            rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                .parse::<alloy_primitives::Address>()
                .expect("Invalid address"),
            btcReimbursementPubKey: H256::from_low_u64_be(103991732982)
                .as_bytes()
                .try_into()
                .expect("Failed to decode btcReimbursementPubKey"),
            utxoScriptPubKey: alloy_primitives::Bytes::from("0x1234567890abcdef"),
        };

        let data = hex::encode(&expected_event.encode_log_data().data);
        let topics = expected_event
            .encode_topics()
            .iter()
            .map(|t| hex::encode(t))
            .collect();

        let log_event = LogEvent::new(data, topics);
        let log_info = LogInfo::new(
            generate_fake_address(1),
            expected_block_hash.into(),
            expected_block_num.into(),
            H256::random().to_string(),
            1,
            false,
        );

        let rsk_log = RskLog::new(log_info, log_event);

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::RegisteredPegInRequest(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
            }
            _ => panic!("Expected RegisteredPegInRequest event"),
        }
    }
}
