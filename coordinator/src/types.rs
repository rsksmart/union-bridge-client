use std::collections::HashMap;
use std::hash::Hash;

use alloy_primitives::{B256, FixedBytes};
#[cfg(test)]
use alloy_sol_types::SolEvent;
use alloy_sol_types::SolEventInterface;
use anyhow::{Result, anyhow};
use bitcoin::PublicKey;
use common::mocks::fake_contracts::FakePegManager::{
    AdvanceFunds, FakePegManagerEvents, RequestAdvanceFunds,
};
use common::msg_broker::bitvmx_types::{
    PartialUtxo, ParticipantRole, PegOutAccepted, PeginAcceptedMessage,
};
use common::types::{Address, BlockHash, BlockNumber, Hash256, RskLog, TxHash};
use log::{debug, info, trace, warn};
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use union_contracts::bindings::bitcoin_manager::BitcoinManager::BitcoinManagerEvents;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    AllCommunicationDataReady, CommitteeRegistryEvents, MemberInfoDeposited, NewCommittee,
    NewPendingCommittee,
};
use union_contracts::bindings::member_registry::MemberRegistry::MemberRegistryEvents;
use union_contracts::bindings::peg_manager::PegManager::{
    OperatorTakeTriggered, PegManagerEvents, PeginAccepted, PeginRequested, PegoutRegistered,
    PegoutRequested,
};
#[cfg(test)]
use union_contracts::bindings::signature_manager::SignatureManager::{
    AllNoncesReady, AllSignaturesReady,
};
use union_contracts::bindings::signature_manager::SignatureManager::{
    AllOperatorTakeTxidsAdded, SignatureManagerEvents,
};
use union_contracts::bindings::stream_manager::StreamManager::StreamManagerEvents;

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
    OperatorTakeTriggered(OperatorTakeTriggeredEvent),
    RemoveRegisteredPeginRequest(PeginRequestedEvent),
    AllNoncesReady(AllNoncesReadyEvent),
    AllSignaturesReady(AllSignaturesReadyEvent),
    AllOperatorTakeTxidsAdded(AllOperatorTakeTxidsAddedEvent),
    NewCommitteePending(NewCommitteePendingEvent),
    NewCommitteeReady(NewCommitteeReadyEvent),
    AllCommunicationDataReady(AllCommunicationDataReadyEvent),
    MemberInfoDeposited(MemberInfoDepositedEvent),
    IgnoredEvent,
    UnknownEvent,
}

#[derive(Debug, Deserialize)]
pub enum UserRequests {
    GetBitVMXFundingAddress,
    ApplyToStream(ApplyToStream),
}

pub type RequestAdvanceFundsEvent = EventWithBlock<RequestAdvanceFunds>;
pub type AdvanceFundsEvent = EventWithBlock<AdvanceFunds>;
pub type PeginRequestedEvent = EventWithBlock<PeginRequested>;
pub type PeginAcceptedEvent = EventWithBlock<PeginAccepted>;
pub type AllNoncesReadyEvent = EventWithBlock<Hash256>;
pub type AllSignaturesReadyEvent = EventWithBlock<Hash256>;
pub type AllOperatorTakeTxidsAddedEvent = EventWithBlock<AllOperatorTakeTxidsAdded>;
pub type PegoutRequestedEvent = EventWithBlock<PegoutRequested>;
pub type PegoutRegisteredEvent = EventWithBlock<PegoutRegistered>;
pub type OperatorTakeTriggeredEvent = EventWithBlock<OperatorTakeTriggered>;
pub type NewCommitteePendingEvent = EventWithBlock<NewPendingCommittee>;
pub type NewCommitteeReadyEvent = EventWithBlock<NewCommittee>;
pub type AllCommunicationDataReadyEvent = EventWithBlock<AllCommunicationDataReady>;
pub type MemberInfoDepositedEvent = EventWithBlock<MemberInfoDeposited>;

pub type EventStatus = bool;

#[derive(Eq, PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct EventWithBlock<T> {
    pub inner: T,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
    pub removed: EventStatus,
    pub tx_hash: TxHash,
}

pub struct EventDecoder;

impl EventDecoder {
    pub fn new() -> Self {
        Self
    }

    /// Extract short variant name from Debug-formatted enum (e.g., `"MemberRegistered"` from `"MemberRegistered(...)"`)
    fn event_variant_name(event: &dyn std::fmt::Debug) -> String {
        let debug_str = format!("{event:?}");
        // split('(') always returns at least one element (the string itself if no '(' is found)
        // so next() will always return Some(&str), making unwrap_or safe here
        let variant = debug_str.split('(').next().unwrap_or("UnknownEvent");
        variant.to_string()
    }

    /// Decodes an RSK log into a `RskPegManagerEvents`.
    ///
    /// Returns:
    /// - `RskPegManagerEvents::UnknownEvent` for malformed logs (no topics)
    /// - `RskPegManagerEvents::UnknownEvent` for logs that don't match any known contract event types
    /// - `RskPegManagerEvents::UnknownEvent` for events that match contract types but have no handler
    /// - Specific event variant for successfully decoded and handled events
    pub fn decode(log: &RskLog) -> RskPegManagerEvents {
        let parsed_topics: Vec<B256> =
            log.event().topics().iter().map(|topic| B256::from(*topic)).collect();

        // Early validation for malformed logs
        if parsed_topics.is_empty() {
            warn!(
                "Log has no topics (malformed or not an event): block_number={:?}, tx_hash={:?}",
                log.info().block_number(),
                log.info().tx_hash()
            );
            return RskPegManagerEvents::UnknownEvent;
        }

        // Try each contract event type and use the first successful decode
        if let Some(event) = Self::try_fake_peg_manager_events(log) {
            return event;
        }

        if let Some(event) = Self::try_peg_manager_events(log) {
            return event;
        }

        if let Some(event) = Self::try_committee_registry_events(log) {
            return event;
        }

        if let Some(event) = Self::try_member_registry_events(log) {
            return event;
        }

        if let Some(event) = Self::try_stream_manager_events(log) {
            return event;
        }

        if let Some(event) = Self::try_signature_manager_events(log) {
            return event;
        }

        if let Some(event) = Self::try_bitcoin_manager_events(log) {
            return event;
        }

        // No decoding succeeded
        warn!(
            "No decoding succeeded for log from {}. topic0: {:?}, tx_hash: {:?}, check if the contract is subscribed to for event decoding in the coordinator",
            log.info().address(),
            log.event().topics().first(),
            log.info().tx_hash()
        );
        RskPegManagerEvents::UnknownEvent
    }

    /// Helper method to extract common log fields
    fn extract_log_fields(
        log: &RskLog,
    ) -> (Vec<B256>, Vec<u8>, BlockNumber, BlockHash, bool, TxHash) {
        let parsed_topics: Vec<B256> =
            log.event().topics().iter().map(|topic| B256::from(*topic)).collect();
        let data = log.event().data().as_bytes().to_vec();
        let block_num = log.info().block_number();
        let block_hash = log.info().block_hash();
        let removed = log.info().removed();
        let tx_hash = log.info().tx_hash();
        (parsed_topics, data, block_num, block_hash, removed, tx_hash)
    }

    fn try_fake_peg_manager_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, block_hash, removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(fpm) = FakePegManagerEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded FakePegManagerEvents: {fpm:?}");
            return Some(Self::convert_fake_peg_manager_event(
                fpm, block_num, block_hash, removed, tx_hash,
            ));
        }
        None
    }

    fn try_peg_manager_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, block_hash, removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(pm) = PegManagerEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded PegManagerEvents: {pm:?}");
            return Some(Self::convert_peg_manager_event(
                pm, block_num, block_hash, removed, tx_hash,
            ));
        }
        None
    }

    fn try_committee_registry_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, block_hash, removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(cr) = CommitteeRegistryEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded CommitteeRegistryEvents: {cr:?}");
            return Some(Self::convert_committee_registry_event(
                cr, block_num, block_hash, removed, tx_hash,
            ));
        }
        None
    }

    fn try_member_registry_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, _block_hash, _removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(mr) = MemberRegistryEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded MemberRegistryEvents: {mr:?}");
            return Some(Self::convert_member_registry_event(&mr, block_num, tx_hash));
        }
        None
    }

    fn try_stream_manager_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, _block_hash, _removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(sm) = StreamManagerEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded StreamManagerEvents: {sm:?}");
            return Some(Self::convert_stream_manager_event(&sm, block_num, tx_hash));
        }
        None
    }

    fn try_signature_manager_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, block_hash, removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(sig) = SignatureManagerEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded SignatureManagerEvents: {sig:?}");
            return Some(Self::convert_signature_manager_event(
                sig, block_num, block_hash, removed, tx_hash,
            ));
        }
        None
    }

    fn try_bitcoin_manager_events(log: &RskLog) -> Option<RskPegManagerEvents> {
        let (parsed_topics, data, block_num, _block_hash, _removed, tx_hash) =
            Self::extract_log_fields(log);

        if let Ok(bm) = BitcoinManagerEvents::decode_raw_log(&parsed_topics, &data) {
            trace!("Decoded BitcoinManagerEvents: {bm:?}");
            return Some(Self::convert_bitcoin_manager_event(&bm, block_num, tx_hash));
        }
        None
    }

    fn convert_peg_manager_event(
        event: PegManagerEvents,
        block_num: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match event {
            PegManagerEvents::PeginRequested(inner) => {
                RskPegManagerEvents::PeginRequested(PeginRequestedEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            PegManagerEvents::PeginAccepted(inner) => {
                RskPegManagerEvents::PeginAccepted(PeginAcceptedEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            PegManagerEvents::PegoutRegistered(inner) => {
                RskPegManagerEvents::PegoutRegistered(PegoutRegisteredEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            PegManagerEvents::PegoutRequested(inner) => {
                RskPegManagerEvents::PegoutRequested(PegoutRequestedEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            PegManagerEvents::OperatorTakeTriggered(inner) => {
                RskPegManagerEvents::OperatorTakeTriggered(OperatorTakeTriggeredEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            event => {
                let variant = Self::event_variant_name(&event);
                debug!("Ignored PegManager event ({variant}): block={block_num}, tx={tx_hash}");
                RskPegManagerEvents::IgnoredEvent
            }
        }
    }

    fn convert_fake_peg_manager_event(
        event: FakePegManagerEvents,
        block_num: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match event {
            FakePegManagerEvents::RequestAdvanceFunds(inner) => {
                RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            FakePegManagerEvents::AdvanceFunds(inner) => {
                RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            FakePegManagerEvents::CheckForkComplete(_inner) => {
                info!("Ignored FakePegManager CheckForkComplete event");
                RskPegManagerEvents::IgnoredEvent
            }
        }
    }

    fn convert_committee_registry_event(
        event: CommitteeRegistryEvents,
        block_num: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match event {
            CommitteeRegistryEvents::NewPendingCommittee(inner) => {
                RskPegManagerEvents::NewCommitteePending(NewCommitteePendingEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            CommitteeRegistryEvents::NewCommittee(inner) => {
                RskPegManagerEvents::NewCommitteeReady(NewCommitteeReadyEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            CommitteeRegistryEvents::AllCommunicationDataReady(inner) => {
                RskPegManagerEvents::AllCommunicationDataReady(AllCommunicationDataReadyEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            event => {
                let variant = Self::event_variant_name(&event);
                debug!(
                    "Ignored CommitteeRegistry event ({variant}): block={block_num}, tx={tx_hash}"
                );
                RskPegManagerEvents::IgnoredEvent
            }
        }
    }

    fn convert_member_registry_event(
        event: &MemberRegistryEvents,
        block_num: BlockNumber,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        let variant = Self::event_variant_name(event);
        debug!("Ignored MemberRegistry event ({variant}): block={block_num}, tx={tx_hash}");
        RskPegManagerEvents::IgnoredEvent
    }

    fn convert_stream_manager_event(
        event: &StreamManagerEvents,
        block_num: BlockNumber,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        // StreamManager events don't have specific handlers yet
        // TODO: Implement proper conversion when event variants are identified
        let variant = Self::event_variant_name(event);
        debug!("Ignored StreamManager event ({variant}): block={block_num}, tx={tx_hash}");
        RskPegManagerEvents::IgnoredEvent
    }

    fn convert_signature_manager_event(
        event: SignatureManagerEvents,
        block_num: BlockNumber,
        block_hash: BlockHash,
        removed: bool,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        match event {
            SignatureManagerEvents::AllNoncesReady(inner) => {
                RskPegManagerEvents::AllNoncesReady(AllNoncesReadyEvent {
                    inner: Hash256::from(inner.hashToSign),
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            SignatureManagerEvents::AllSignaturesReady(inner) => {
                RskPegManagerEvents::AllSignaturesReady(AllSignaturesReadyEvent {
                    inner: Hash256::from(inner.hashToSign),
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            SignatureManagerEvents::AllOperatorTakeTxidsAdded(inner) => {
                RskPegManagerEvents::AllOperatorTakeTxidsAdded(AllOperatorTakeTxidsAddedEvent {
                    inner,
                    block_number: block_num,
                    block_hash,
                    removed,
                    tx_hash,
                })
            }
            event => {
                let variant = Self::event_variant_name(&event);
                debug!(
                    "Ignored SignatureManager event ({variant}): block={block_num}, tx={tx_hash}"
                );
                RskPegManagerEvents::IgnoredEvent
            }
        }
    }

    fn convert_bitcoin_manager_event(
        event: &BitcoinManagerEvents,
        block_num: BlockNumber,
        tx_hash: TxHash,
    ) -> RskPegManagerEvents {
        // BitcoinManager events don't have specific handlers yet
        // TODO: Implement proper conversion when event variants are identified
        let variant = Self::event_variant_name(event);
        debug!("Ignored BitcoinManager event ({variant}): block={block_num}, tx={tx_hash}");
        RskPegManagerEvents::IgnoredEvent
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
            hash_to_sign: Hash256::from(FixedBytes::from(
                <[u8; 32]>::try_from(value.user_take_sighash)
                    .map_err(|_| anyhow!("Hash must be exactly 32 bytes"))?,
            )),
            nonce: value.user_take_nonce,
            signature: value.user_take_signature,
        })
    }
}

impl TryFrom<PeginAcceptedMessage> for RegisterSignaturesBitVmxData {
    type Error = anyhow::Error;

    fn try_from(value: PeginAcceptedMessage) -> Result<Self, Self::Error> {
        Ok(RegisterSignaturesBitVmxData {
            hash_to_sign: Hash256::from(FixedBytes::from(
                <[u8; 32]>::try_from(value.accept_pegin_sighash)
                    .map_err(|_| anyhow!("Hash must be exactly 32 bytes"))?,
            )),
            nonce: value.accept_pegin_nonce,
            signature: value.accept_pegin_signature,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct TickScheduler<K: Eq + Hash + Clone> {
    pending: HashMap<K, u32>,
}

impl<K: Eq + Hash + Clone> TickScheduler<K> {
    pub fn new() -> Self {
        Self { pending: HashMap::new() }
    }

    pub fn schedule(&mut self, id: K, delay_ticks: u32) {
        self.pending.insert(id, delay_ticks);
    }

    pub fn cancel(&mut self, id: &K) {
        self.pending.remove(id);
    }

    pub fn is_scheduled(&self, id: &K) -> bool {
        self.pending.contains_key(id)
    }

    pub fn tick(&mut self) -> Vec<K> {
        let mut ready: Vec<K> = Vec::new();
        for (id, ticks) in &mut self.pending {
            if *ticks > 0 {
                *ticks -= 1;
            }
            if *ticks == 0 {
                ready.push(id.clone());
            }
        }
        for id in &ready {
            self.pending.remove(id);
        }
        ready
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// Time-based scheduler that uses block timestamps to track expiration times
pub(crate) struct TimeBasedScheduler<K: Eq + Hash + Clone> {
    pending: HashMap<K, u64>, // flow_id -> expiration_timestamp (in seconds)
}

impl<K: Eq + Hash + Clone> TimeBasedScheduler<K> {
    pub fn new() -> Self {
        Self { pending: HashMap::new() }
    }

    /// Schedule a timeout for the given id, expiring after the specified duration in seconds
    /// from the current block timestamp
    pub fn schedule(&mut self, id: K, current_timestamp: u64, timeout_seconds: u64) {
        let expiration_timestamp = current_timestamp + timeout_seconds;
        self.pending.insert(id, expiration_timestamp);
    }

    pub fn cancel(&mut self, id: &K) {
        self.pending.remove(id);
    }

    pub fn is_scheduled(&self, id: &K) -> bool {
        self.pending.contains_key(id)
    }

    /// Check for expired timeouts based on the current block timestamp
    /// Returns a vector of expired ids
    pub fn check_expired(&mut self, current_timestamp: u64) -> Vec<K> {
        let mut expired: Vec<K> = Vec::new();
        let mut to_remove: Vec<K> = Vec::new();

        for (id, expiration_timestamp) in &self.pending {
            if current_timestamp >= *expiration_timestamp {
                expired.push(id.clone());
                to_remove.push(id.clone());
            }
        }

        for id in &to_remove {
            self.pending.remove(id);
        }

        expired
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Utxo {
    // temporarily omitted for Regtest stage
    // pub txid: String,
    // temporarily omitted for Regtest stage
    // pub vout: u32,
    pub value: u64,
}

// temporarily omitted for Regtest stage
// impl TryFrom<Utxo> for UTXO {
//     type Error = anyhow::Error;
//
//     fn try_from(utxo: Utxo) -> Result<Self, Self::Error> {
//         Ok(UTXO {
//             txid: utxo
//                 .txid
//                 .parse()
//                 .context("Failed to parse String txid to FixedBytes<32>")?,
//             outputIndex: utxo.vout,
//             amount: utxo.value,
//         })
//     }
// }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberOfCommittee {
    pub address: Address,
    pub role: ParticipantRole,
    pub take_key: PublicKey,
    pub dispute_key: PublicKey,
    pub funding_utxo: PartialUtxo,
    pub committee_idx: usize,
}

#[cfg(test)]
mod tests {
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
    use uuid::Uuid;

    use super::*;

    fn create_rsk_log_from_event<T: SolEvent>(
        event: &T,
        block_hash: H256,
        block_number: u64,
        removed: bool,
    ) -> (Hash256, RskLog) {
        let data = DataBytes::new(event.encode_log_data().data.to_vec());
        let topics = event.encode_topics().iter().map(|t| Hash256::from(B256::from(*t))).collect();

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
    fn test_exhaustive_decoder_with_committee_event() {
        // Create a committee event
        let expected_event = AllCommunicationDataReady { _committeeId: 12345 };

        let (_, log) =
            create_rsk_log_from_event(&expected_event, H256::from_low_u64_be(123), 456, false);

        let result = EventDecoder::decode(&log);

        // Should successfully decode using exhaustive method
        match result {
            RskPegManagerEvents::AllCommunicationDataReady(event) => {
                assert_eq!(event.inner._committeeId, 12345);
            }
            _ => panic!("Expected AllCommunicationDataReady event"),
        }
    }

    #[test]
    fn test_exhaustive_decoder_with_pegin_event() {
        // Create a pegin event
        let expected_event = PeginRequested {
            committeeId: 99u128,
            requestPeginTxid: H256::from_low_u64_be(111)
                .as_bytes()
                .try_into()
                .expect("Failed to decode requestPeginTxid"),
            acceptPeginTxid: H256::from_low_u64_be(222)
                .as_bytes()
                .try_into()
                .expect("Failed to decode acceptPeginTxid"),
            vout: 1,
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 0u8,
            },
            requestPeginInfo: RequestPeginTempInfo {
                rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                    .parse::<alloy_primitives::Address>()
                    .expect("Invalid address"),
                btcReimbursementPubKey: H256::from_low_u64_be(103_991_732_982)
                    .as_bytes()
                    .try_into()
                    .expect("Failed to decode key"),
                acceptPeginSignatureHash: H256::from_low_u64_be(4_444_444)
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

        let (_, log) =
            create_rsk_log_from_event(&expected_event, H256::from_low_u64_be(123), 456, false);

        let result = EventDecoder::decode(&log);

        // Should successfully decode using exhaustive method
        match result {
            RskPegManagerEvents::PeginRequested(event) => {
                assert_eq!(event.inner.committeeId, 99u128);
            }
            _ => panic!("Expected PeginRequested event"),
        }
    }

    #[test]
    fn test_exhaustive_decoder_fallback_behavior() {
        // Create a completely unknown event
        let log = FakeLogGenerator::new()
            .generate_log("UnknownEvent(uint256,string)", generate_fake_address(1));

        let result = EventDecoder::decode(&log);

        // Should fall back to topic-based decoding and return UnknownEvent
        assert!(matches!(result, RskPegManagerEvents::UnknownEvent));
    }

    #[test]
    fn test_decode_unknown_event() {
        let log = FakeLogGenerator::new()
            .generate_log("Transfer(address,address,uint256)", generate_fake_address(1));

        let result = EventDecoder::decode(&log);
        assert_eq!(result, RskPegManagerEvents::UnknownEvent);
    }

    #[test]
    fn test_decode_invalid_data() {
        let log_event: LogEvent = LogEvent::new(
            DataBytes::new("fake".as_bytes().to_vec()),
            vec![event_signature_to_topic("Transfer(address,address,uint256)")],
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

        let result = EventDecoder::decode(&log);
        assert_eq!(result, RskPegManagerEvents::UnknownEvent);
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

        let result = EventDecoder::decode(&log);
        assert_eq!(result, RskPegManagerEvents::UnknownEvent);
    }

    #[test]
    fn test_decode_invalid_topics() {
        let topic = event_signature_to_topic("Transfer(address,address,uint256)");
        let log_event: LogEvent = LogEvent::new(
            DataBytes::from_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap(),
            vec![topic, topic, topic, topic, topic], // 5 topics, invalid
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

        let result = EventDecoder::decode(&log);
        assert_eq!(result, RskPegManagerEvents::UnknownEvent);
    }

    #[test]
    fn test_decode_pegin_requested_event() {
        let expected_block_hash = H256::from_low_u64_be(123);
        let expected_block_num = 789;

        let expected_event = PeginRequested {
            committeeId: 99u128,
            requestPeginTxid: H256::from_low_u64_be(111)
                .as_bytes()
                .try_into()
                .expect("Failed to decode requestPeginTxid"),
            acceptPeginTxid: H256::from_low_u64_be(222)
                .as_bytes()
                .try_into()
                .expect("Failed to decode acceptPeginTxid"),
            vout: 1,
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 0u8,
            },
            requestPeginInfo: RequestPeginTempInfo {
                rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                    .parse::<alloy_primitives::Address>()
                    .expect("Invalid address"),
                btcReimbursementPubKey: H256::from_low_u64_be(103_991_732_982)
                    .as_bytes()
                    .try_into()
                    .expect("Failed to decode key"),
                acceptPeginSignatureHash: H256::from_low_u64_be(4_444_444)
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

        let result = EventDecoder::decode(&rsk_log);
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
            acceptPeginTxid: FixedBytes::<32>::from_slice(H256::from_low_u64_be(2).as_bytes()),
            peginRequestTxid: FixedBytes::<32>::from_slice(H256::from_low_u64_be(3).as_bytes()),
            vout: 0,
            streamPosition: StreamPosition {
                streamId: 42,
                packetNumber: 33,
                slotId: 0,
                pegStatus: 1u8,
            },
            speedUpPubKey: FixedBytes::<32>::from_slice(
                H256::from_low_u64_be(103_991_732_982).as_bytes(),
            ),
            rskDestinationAddress: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
                .parse::<Address>()
                .expect("Invalid address"),
            rbtcAmount: U256::from(12_345_678),
            utxoScriptPubKey: Bytes::from("0xabcdef0123456789"),
        };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );
        let result = EventDecoder::decode(&rsk_log);
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

        let result = EventDecoder::decode(&rsk_log);
        // Now these events are properly handled through SignatureManagerEvents enum
        match result {
            RskPegManagerEvents::AllNoncesReady(data) => {
                assert_eq!(data.inner, Hash256::from(expected_hash_to_sign));
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert!(!data.removed);
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

        let result = EventDecoder::decode(&rsk_log);
        // Now these events are properly handled through SignatureManagerEvents enum
        match result {
            RskPegManagerEvents::AllSignaturesReady(data) => {
                assert_eq!(data.inner, Hash256::from(expected_hash_to_sign));
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert!(data.removed);
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

        let expected_event = AllOperatorTakeTxidsAdded {
            acceptPeginTxid: expected_accept_pegin_tx_hash
                .as_bytes()
                .try_into()
                .expect("Failed to decode acceptPeginTxid"),
        };

        let data = DataBytes::new(expected_event.encode_log_data().data.to_vec());
        let topics =
            expected_event.encode_topics().iter().map(|t| Hash256::from(B256::from(*t))).collect();

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

        let result = EventDecoder::decode(&rsk_log);
        // Now these events are properly handled through SignatureManagerEvents enum
        match result {
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(data) => {
                assert_eq!(data.inner, expected_event);
                assert_eq!(data.block_number, expected_block_num);
                assert_eq!(data.block_hash, expected_block_hash.into());
                assert!(data.removed);
                assert_eq!(data.tx_hash, expected_tx_hash);
            }
            _ => panic!("Expected AllOperatorTakeTxidsAdded event"),
        }
    }

    #[test]
    fn test_tick_scheduler_schedule_and_tick_ready() {
        let mut sched: TickScheduler<Uuid> = TickScheduler::new();
        let id = Uuid::new_v4();
        assert!(sched.is_empty());
        sched.schedule(id, 3);
        assert!(sched.is_scheduled(&id));

        // Tick 1: not ready
        let ready = sched.tick();
        assert!(ready.is_empty());

        // Tick 2: not ready
        let ready = sched.tick();
        assert!(ready.is_empty());

        // Tick 3: now ready
        let ready = sched.tick();
        assert_eq!(ready, vec![id]);
        assert!(!sched.is_scheduled(&id));
    }

    #[test]
    fn test_tick_scheduler_reschedule_overwrites() {
        let mut sched: TickScheduler<Uuid> = TickScheduler::new();
        let id = Uuid::new_v4();
        sched.schedule(id, 5);
        // Overwrite with a shorter delay
        sched.schedule(id, 2);

        // After two ticks it should be ready (not five)
        assert!(sched.tick().is_empty());
        let ready = sched.tick();
        assert_eq!(ready, vec![id]);
    }

    #[test]
    fn test_tick_scheduler_cancel_prevents_ready() {
        let mut sched: TickScheduler<Uuid> = TickScheduler::new();
        let id = Uuid::new_v4();
        sched.schedule(id, 1);
        assert!(sched.is_scheduled(&id));
        sched.cancel(&id);
        assert!(!sched.is_scheduled(&id));
        let ready = sched.tick();
        assert!(ready.is_empty());
    }

    #[test]
    fn test_tick_scheduler_multiple_ids_independent() {
        let mut sched: TickScheduler<Uuid> = TickScheduler::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();
        sched.schedule(id_a, 1);
        sched.schedule(id_b, 3);
        sched.schedule(id_c, 2);

        // After 1st tick: A ready
        let ready = sched.tick();
        assert_eq!(ready, vec![id_a]);

        // After 2nd tick: C ready
        let ready = sched.tick();
        assert_eq!(ready, vec![id_c]);

        // After 3rd tick: B ready
        let ready = sched.tick();
        assert_eq!(ready, vec![id_b]);
    }

    #[test]
    fn test_tick_scheduler_is_empty_and_clear() {
        let mut sched: TickScheduler<Uuid> = TickScheduler::new();
        assert!(sched.is_empty());
        let id = Uuid::new_v4();
        sched.schedule(id, 10);
        assert!(!sched.is_empty());
        sched.clear();
        assert!(sched.is_empty());
    }

    #[test]
    fn test_decode_new_committee_pending_event() {
        let expected_block_hash = H256::from_low_u64_be(456);
        let expected_block_num = 123;

        // create a minimal committee structure
        let committee = Committee {
            aggregatedKey: Bytes::from(H256::from_low_u64_be(12345).as_bytes().to_vec()),
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
            createdAt: U256::default(),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 0,
            fundingUTXOs: vec![],
        };

        let expected_event = NewPendingCommittee { committeeId: 42, _committee: committee };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let result = EventDecoder::decode(&rsk_log);
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
            aggregatedKey: FixedBytes::<32>::from_slice(H256::from_low_u64_be(67890).as_bytes())
                .into(),
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
            createdAt: U256::default(),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 0,
            fundingUTXOs: vec![],
        };

        let expected_event = NewCommittee { committeeId: 99, _committee: committee };

        let removed = true;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let result = EventDecoder::decode(&rsk_log);
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

        let expected_event = AllCommunicationDataReady { _committeeId: expected_committee_id };

        let removed = false;
        let (expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let result = EventDecoder::decode(&rsk_log);
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

    #[test]
    fn test_decode_member_info_deposited_event() {
        let expected_block_hash = H256::from_low_u64_be(1010);
        let expected_block_num = 1111;
        let expected_committee_id = 1212;
        let expected_member_address = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
            .parse::<Address>()
            .expect("invalid address");

        let expected_event = MemberInfoDeposited {
            committeeId: expected_committee_id,
            member: expected_member_address,
            aggregatedKey: alloy_primitives::FixedBytes::<32>::from_slice(
                H256::from_low_u64_be(12345).as_bytes(),
            )
            .into(),
        };

        let removed = true;
        let (_expected_tx_hash, rsk_log) = create_rsk_log_from_event(
            &expected_event,
            expected_block_hash,
            expected_block_num,
            removed,
        );

        let result = EventDecoder::decode(&rsk_log);
        // MemberInfoDeposited is a CommitteeRegistry event but has no handler, so it should return IgnoredEvent
        match result {
            RskPegManagerEvents::IgnoredEvent => {
                // IgnoredEvent no longer carries data - it's just a marker that the event was ignored
            }
            _ => panic!("Expected IgnoredEvent for MemberInfoDeposited"),
        }
    }
}
