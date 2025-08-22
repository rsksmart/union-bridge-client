use crate::types::RskPegManagerEvents::UnknownEvent;
use actors_mocking::fake_contracts::FakePegManager::{AdvanceFunds, RequestAdvanceFunds};
use alloy_primitives::{B256, LogData};
use alloy_sol_types::SolEvent;
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::{P2PAddress, PartialUtxo, ParticipantRole, PegOutAccepted};
use common::types::{Address, BlockHash, BlockNumber, Hash256, RskLog, TxHash};
use log::{error, warn};
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    AllCommunicationDataReady, NewCommittee, NewPendingCommittee,
};
use union_contracts::bindings::peg_manager::PegManager::{
    PegStatus, PeginAccepted, PeginRequested, PegoutRegistered, PegoutRequested, StreamPosition,
};
use union_contracts::bindings::signature_manager::SignatureManager::{
    AllNoncesReady, AllOperatorTakeTxHashesAdded, AllSignaturesReady,
};

use crate::user_requests::ApplyToStream;

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-183

#[derive(Eq, PartialEq, Debug)]
pub enum RskPegManagerEvents {
    RequestAdvanceFunds(RequestAdvanceFundsEvent), // temporarily mock, no need to test it
    AdvanceFunds(AdvanceFundsEvent),               // temporarily mock, no need to test it
    PeginRequested(PeginRequestedEvent),
    PeginAccepted(PeginAcceptedEvent),
    PegoutRegistered(PegoutRegisteredEvent),
    PegoutRequested(PegoutRequestedEvent),
    RemoveRegisteredPeginRequest(PeginRequestedEvent),
    AllNoncesReady(AllNoncesReadyEvent),
    AllSignaturesReady(AllSignaturesReadyEvent),
    AllOperatorTakeTxHashesAdded(AllOperatorTakeTxHashesAddedEvent),
    NewCommitteePending(NewCommitteePendingEvent),
    NewCommitteeReady(NewCommitteeReadyEvent),
    AllCommunicationDataReady(AllCommunicationDataReadyEvent),
    UnknownEvent,
}

#[derive(Debug, Deserialize)]
pub enum UserRequests {
    ApplyToStream(ApplyToStream),
}

pub type RequestAdvanceFundsEvent = EventWithBlock<RequestAdvanceFunds>;
pub type AdvanceFundsEvent = EventWithBlock<AdvanceFunds>;
pub type PeginRequestedEvent = EventWithBlock<PeginRequested>;
pub type PeginAcceptedEvent = EventWithBlock<PeginAccepted>;
pub type AllNoncesReadyEvent = EventWithBlock<Hash256>;
pub type AllSignaturesReadyEvent = EventWithBlock<Hash256>;
pub type AllOperatorTakeTxHashesAddedEvent = EventWithBlock<AllOperatorTakeTxHashesAdded>;
pub type PegoutRequestedEvent = EventWithBlock<PegoutRequested>;
pub type PegoutRegisteredEvent = EventWithBlock<PegoutRegistered>;
pub type NewCommitteePendingEvent = EventWithBlock<NewPendingCommittee>;
pub type NewCommitteeReadyEvent = EventWithBlock<NewCommittee>;
pub type AllCommunicationDataReadyEvent = EventWithBlock<AllCommunicationDataReady>;

pub type EventStatus = bool;
type DecoderFn = fn(&LogData, BlockNumber, BlockHash, EventStatus, TxHash) -> RskPegManagerEvents;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct EventWithBlock<T> {
    pub inner: T,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
    pub removed: EventStatus,
    pub tx_hash: TxHash,
}

pub struct EventDecoder {
    dispatch: HashMap<B256, DecoderFn>,
}

impl EventDecoder {
    pub fn new() -> Self {
        let mut dispatcher = HashMap::new();
        dispatcher.insert(
            PeginRequested::SIGNATURE_HASH,
            Self::decode_pegin_requested_event as DecoderFn,
        );
        dispatcher.insert(
            PeginAccepted::SIGNATURE_HASH,
            Self::decode_pegin_accepted_event as DecoderFn,
        );
        dispatcher.insert(
            RequestAdvanceFunds::SIGNATURE_HASH,
            Self::decode_request_advance_funds_event as DecoderFn,
        );
        dispatcher.insert(
            AdvanceFunds::SIGNATURE_HASH,
            Self::decode_advance_funds_event as DecoderFn,
        );
        dispatcher.insert(
            AllNoncesReady::SIGNATURE_HASH,
            Self::decode_all_nonces_ready_event as DecoderFn,
        );
        dispatcher.insert(
            AllSignaturesReady::SIGNATURE_HASH,
            Self::decode_all_signatures_ready_event as DecoderFn,
        );
        dispatcher.insert(
            AllOperatorTakeTxHashesAdded::SIGNATURE_HASH,
            Self::decode_all_operator_take_tx_hashes_added_event as DecoderFn,
        );
        dispatcher.insert(
            PegoutRegistered::SIGNATURE_HASH,
            Self::decode_pegout_registered_event as DecoderFn,
        );
        dispatcher.insert(
            PegoutRequested::SIGNATURE_HASH,
            Self::decode_pegout_requested_event as DecoderFn,
        );
        dispatcher.insert(
            NewPendingCommittee::SIGNATURE_HASH,
            Self::decode_new_committee_pending_event as DecoderFn,
        );
        dispatcher.insert(
            NewCommittee::SIGNATURE_HASH,
            Self::decode_new_committee_ready_event as DecoderFn,
        );
        dispatcher.insert(
            AllCommunicationDataReady::SIGNATURE_HASH,
            Self::decode_all_communication_data_ready_event as DecoderFn,
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
        let tx_hash = log.info().tx_hash();
        match self.dispatch.get(&topic0) {
            Some(decoder_fn) => decoder_fn(
                &log_data,
                block_num,
                block_hash,
                log.info().removed(),
                tx_hash,
            ),
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
            .map(|topic| B256::from(*topic))
            .collect();

        let hex_data = log.event().data().as_bytes().to_vec();

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

    fn decode_pegin_requested_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: EventStatus,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match PeginRequested::decode_log_data(&log_data) {
            Ok(ev) => RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
                inner: ev,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_pegin_accepted_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: EventStatus,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match PeginAccepted::decode_log_data(&log_data) {
            Ok(ev) => RskPegManagerEvents::PeginAccepted(PeginAcceptedEvent {
                inner: ev,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_pegout_requested_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match PegoutRequested::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::PegoutRequested(PegoutRequestedEvent {
                inner: event,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_pegout_registered_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match PegoutRegistered::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::PegoutRegistered(PegoutRegisteredEvent {
                inner: event,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_request_advance_funds_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: EventStatus,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match RequestAdvanceFunds::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
                inner: event,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_advance_funds_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: EventStatus,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match AdvanceFunds::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: event,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_all_nonces_ready_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match AllNoncesReady::decode_log_data(&log_data) {
            Ok(event) => {
                // TODO: Replace with proper error handling
                RskPegManagerEvents::AllNoncesReady(AllNoncesReadyEvent {
                    inner: Hash256::from(event.hashToSign),
                    block_number,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            Err(_) => RskPegManagerEvents::UnknownEvent,
        }
    }

    fn decode_all_signatures_ready_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match AllSignaturesReady::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::AllSignaturesReady(AllSignaturesReadyEvent {
                inner: Hash256::from(event.hashToSign),
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_all_operator_take_tx_hashes_added_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match AllOperatorTakeTxHashesAdded::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::AllOperatorTakeTxHashesAdded(
                AllOperatorTakeTxHashesAddedEvent {
                    inner: event,
                    block_number,
                    block_hash,
                    removed,
                    tx_hash,
                },
            ),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_new_committee_pending_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match NewPendingCommittee::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::NewCommitteePending(NewCommitteePendingEvent {
                inner: event,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_new_committee_ready_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match NewCommittee::decode_log_data(&log_data) {
            Ok(event) => RskPegManagerEvents::NewCommitteeReady(NewCommitteeReadyEvent {
                inner: event,
                block_number,
                block_hash,
                removed,
                tx_hash,
            }),
            Err(_) => UnknownEvent,
        }
    }

    fn decode_all_communication_data_ready_event(
        log_data: &LogData,
        block_number: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match AllCommunicationDataReady::decode_log_data(&log_data) {
            Ok(event) => {
                RskPegManagerEvents::AllCommunicationDataReady(AllCommunicationDataReadyEvent {
                    inner: event,
                    block_number,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            Err(_) => UnknownEvent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSignaturesBitVmxData {
    pub hash_to_sign: Hash256,
    pub nonce: PubNonce,
    pub signature: PartialSignature,
}

impl TryFrom<PegOutAccepted> for RegisterSignaturesBitVmxData {
    type Error = anyhow::Error;

    fn try_from(value: PegOutAccepted) -> Result<Self, Self::Error> {
        Ok(RegisterSignaturesBitVmxData {
            hash_to_sign: Hash256::from(alloy_primitives::FixedBytes::from(
                <[u8; 32]>::try_from(value.user_take_sighash)
                    .map_err(|_| anyhow::anyhow!("Hash must be exactly 32 bytes"))?,
            )),
            nonce: value.user_take_nonce,
            signature: value.user_take_signature,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberOfCommittee {
    pub address: Address,
    pub role: ParticipantRole,
    pub take_key: PublicKey,
    pub dispute_key: PublicKey,
    pub p2p_addrs: Vec<P2PAddress>,
    pub funding_utxo: PartialUtxo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256, Bytes, FixedBytes, U256};
    use common::test_utils::rsk_log_generator::{FakeLogGenerator, event_signature_to_topic};
    use common::test_utils::rsk_utils::generate_fake_address;
    use common::types::{BlockHash, DataBytes, Hash256, LogEvent, LogInfo, RskLog, TxHash};
    use primitive_types::H256;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        Committee, CommitteeMember,
    };
    use union_contracts::bindings::peg_manager::PegManager::{
        PrevoutData, RequestPeginTempInfo, StreamPosition,
    };

    fn create_rsk_log_from_event<T: SolEvent>(
        event: &T,
        block_hash: H256,
        block_number: u64,
        removed: bool,
    ) -> (Hash256, RskLog) {
        let data = DataBytes::new(event.encode_log_data().data.to_vec());
        let topics = event
            .encode_topics()
            .iter()
            .map(|t| Hash256::from(B256::from(*t)))
            .collect();

        let log_event = LogEvent::new(data, topics);
        let tx_hash = TxHash::from(H256::random());
        let log_info = LogInfo::new(
            generate_fake_address(1),
            block_hash.into(),
            block_number.into(),
            tx_hash,
            1,
            removed,
        );

        (tx_hash, RskLog::new(log_info, log_event))
    }

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
            DataBytes::new("fake".as_bytes().to_vec()),
            vec![event_signature_to_topic(
                "Transfer(address,address,uint256)",
            )],
        );

        let log_info = LogInfo::new(
            generate_fake_address(1),
            BlockHash::from(H256::random()),
            1.into(),
            TxHash::from(H256::random()),
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
            DataBytes::from_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap(),
            vec![],
        );

        let log_info = LogInfo::new(
            generate_fake_address(1),
            BlockHash::from(H256::random()),
            1.into(),
            TxHash::from(H256::random()),
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
            DataBytes::from_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap(),
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
            TxHash::from(H256::random()),
            1,
            false,
        );

        let log = RskLog::new(log_info, log_event);

        let decoder = EventDecoder::new();
        let result = decoder.decode(log);
        assert_eq!(result, UnknownEvent);
    }

    #[test]
    fn test_decode_pegin_requested_event() {
        let expected_block_hash = H256::from_low_u64_be(123);
        let expected_block_num = 789;

        let expected_event = PeginRequested {
            committeeId: U256::from(99),
            requestPeginTxHash: H256::from_low_u64_be(111)
                .as_bytes()
                .try_into()
                .expect("Failed to decode requestPeginTxHash"),
            acceptPeginTxHash: H256::from_low_u64_be(222)
                .as_bytes()
                .try_into()
                .expect("Failed to decode acceptPeginTxHash"),
            vout: 1,
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 0.into(),
            },
            requestPeginInfo: RequestPeginTempInfo {
                rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                    .parse::<alloy_primitives::Address>()
                    .expect("Invalid address"),
                btcReimbursementPubKey: H256::from_low_u64_be(103991732982)
                    .as_bytes()
                    .try_into()
                    .expect("Failed to decode key"),
                acceptPeginSignatureHash: H256::from_low_u64_be(4444444)
                    .as_bytes()
                    .try_into()
                    .expect("Failed to decode hash"),
            },
            prevoutData: PrevoutData {
                value: 1000,
                scriptPubKey: alloy_primitives::Bytes::from("0x1234567890abcdef"),
            },
            acceptPeginSignatureMessage: alloy_primitives::Bytes::from("0xabcdef0123456789"),
        };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::PeginRequested(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, removed);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected PeginRequested event"),
        }
    }

    #[test]
    fn test_decode_pegin_accepted_event() {
        let expected_block_hash = H256::from_low_u64_be(123);
        let expected_block_num = 789;

        let expected_event = PeginAccepted {
            blockHash: FixedBytes::<32>::from_slice(H256::from_low_u64_be(1).as_bytes()),
            acceptPeginTxHash: FixedBytes::<32>::from_slice(H256::from_low_u64_be(2).as_bytes()),
            peginRequestTxHash: FixedBytes::<32>::from_slice(H256::from_low_u64_be(3).as_bytes()),
            vout: 0,
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 1.into(),
            },
            speedUpPubKey: FixedBytes::<32>::from_slice(
                H256::from_low_u64_be(103991732982).as_bytes(),
            ),
            rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                .parse::<Address>()
                .expect("Invalid address"),
            rbtcAmount: U256::from(12345678),
            utxoScriptPubKey: Bytes::from("0xabcdef0123456789"),
        };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );
        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::PeginAccepted(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, removed);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected PeginAccepted event"),
        }
    }

    #[test]
    fn test_decode_all_nonces_ready_event() {
        let expected_block_hash = H256::from_low_u64_be(456);
        let expected_block_num = 123;
        let expected_hash_to_sign = H256::from_low_u64_be(789);

        let expected_event = AllNoncesReady {
            hashToSign: expected_hash_to_sign
                .as_bytes()
                .try_into()
                .expect("Failed to decode hashToSign"),
        };

        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            false,
        );

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::AllNoncesReady(data) => {
                assert_eq!(data.inner, Hash256::from(expected_hash_to_sign));
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected AllNoncesReady event"),
        }
    }

    #[test]
    fn test_decode_all_signatures_ready_event() {
        let expected_block_hash = H256::from_low_u64_be(999);
        let expected_block_num = 555;
        let expected_hash_to_sign = H256::from_low_u64_be(1111);

        let expected_event = AllSignaturesReady {
            hashToSign: expected_hash_to_sign
                .as_bytes()
                .try_into()
                .expect("Failed to decode hashToSign"),
        };

        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            true,
        );

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::AllSignaturesReady(data) => {
                assert_eq!(data.inner, Hash256::from(expected_hash_to_sign));
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, true);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected AllSignaturesReady event"),
        }
    }

    #[test]
    fn test_decode_all_operator_take_tx_hashes_added_event() {
        let expected_block_hash = H256::from_low_u64_be(777);
        let expected_block_num = 333;
        let expected_accept_pegin_tx_hash = H256::from_low_u64_be(555);

        let expected_event = AllOperatorTakeTxHashesAdded {
            acceptPeginTxHash: expected_accept_pegin_tx_hash
                .as_bytes()
                .try_into()
                .expect("Failed to decode acceptPeginTxHash"),
        };

        let data = DataBytes::new(expected_event.encode_log_data().data.to_vec());
        let topics = expected_event
            .encode_topics()
            .iter()
            .map(|t| Hash256::from(B256::from(*t)))
            .collect();

        let log_event = LogEvent::new(data, topics);
        let expected_tx_hash = TxHash::from(H256::random());
        let log_info = LogInfo::new(
            generate_fake_address(1),
            expected_block_hash.into(),
            expected_block_num.into(),
            expected_tx_hash,
            1,
            true,
        );

        let rsk_log = RskLog::new(log_info, log_event);

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::AllOperatorTakeTxHashesAdded(data) => {
                assert_eq!(
                    data.inner.acceptPeginTxHash.as_slice(),
                    expected_accept_pegin_tx_hash.as_bytes()
                );
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, true);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected AllOperatorTakeTxHashesAdded event"),
        }
    }

    #[test]
    fn test_decode_new_committee_pending_event() {
        let expected_block_hash = H256::from_low_u64_be(456);
        let expected_block_num = 123;

        // create a minimal committee structure
        let committee = Committee {
            aggregatedKey: FixedBytes::<32>::from_slice(H256::from_low_u64_be(12345).as_bytes()),
            members: vec![CommitteeMember {
                memberAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                    .parse::<Address>()
                    .expect("Invalid address"),
                role: 1, // assuming role is u8, using 1 for operator or similar
            }],
            leaderAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                .parse::<Address>()
                .expect("Invalid address"),
            operatorTakeIndex: U256::from(0),
            createdAt: Default::default(),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 0,
        };

        let expected_event = NewPendingCommittee {
            committeeId: 42,
            _committee: committee,
        };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::NewCommitteePending(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, removed);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected NewCommitteePending event"),
        }
    }

    #[test]
    fn test_decode_new_committee_ready_event() {
        let expected_block_hash = H256::from_low_u64_be(789);
        let expected_block_num = 456;

        // create a minimal committee structure
        let committee = Committee {
            aggregatedKey: FixedBytes::<32>::from_slice(H256::from_low_u64_be(67890).as_bytes()),
            members: vec![
                CommitteeMember {
                    memberAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                        .parse::<Address>()
                        .expect("Invalid address"),
                    role: 1, // operator role
                },
                CommitteeMember {
                    memberAddress: "0x8ba1f109551bD432803012645aac136c22C57Bef"
                        .parse::<Address>()
                        .expect("Invalid address"),
                    role: 2, // watchtower role
                },
            ],
            leaderAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                .parse::<Address>()
                .expect("Invalid address"),
            operatorTakeIndex: U256::from(1),
            createdAt: Default::default(),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 0,
        };

        let expected_event = NewCommittee {
            committeeId: 99,
            _committee: committee,
        };

        let removed = true;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::NewCommitteeReady(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, removed);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected NewCommitteeReady event"),
        }
    }

    #[test]
    fn test_decode_all_communication_data_ready_event() {
        let expected_block_hash = H256::from_low_u64_be(333);
        let expected_block_num = 777;
        let expected_committee_id = 12345;

        let expected_event = AllCommunicationDataReady {
            _committeeId: expected_committee_id,
        };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let decoder = EventDecoder::new();
        let result = decoder.decode(rsk_log);
        match result {
            RskPegManagerEvents::AllCommunicationDataReady(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert_eq!(data.removed, removed);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected AllCommunicationDataReady event"),
        }
    }
}
