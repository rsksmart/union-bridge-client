use std::rc::Rc;

use anyhow::{Context, Result};
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::{
    ADVANCE_FUNDS_INPUT, Committee, CommsAddress, DisputeCoreData, IncomingBitVMXApiMessages,
    MemberData, PartialUtxo, Utxo, VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use log::{debug, info};
use uuid::Uuid;

use crate::flows::committee::common::{CommitteeData, send_bitvmx_msg};
use crate::flows::committee::setup_committee_flow::NO_LEADER_IDX;

const PROGRAM_TYPE_DISPUTE_CORE: &str = "dispute_core";

#[derive(Clone, Copy)]
pub struct AggregatedKeys {
    pub take: PublicKey,
    pub dispute: PublicKey,
}

pub struct DisputeCoreSetup<BC: BitVmxBrokerClientApi> {
    broker_client: Rc<BC>,
}

impl<BC: BitVmxBrokerClientApi> DisputeCoreSetup<BC> {
    pub fn new(broker_client: Rc<BC>) -> Self {
        Self { broker_client }
    }

    pub fn setup(
        &self,
        committee_data: &CommitteeData,
        p2p_addresses: &[CommsAddress],
        aggregated_keys: AggregatedKeys,
        my_speedup_funding_utxo: Utxo,
        stream_denomination: u64,
        advance_funds_utxo: PartialUtxo,
    ) -> Result<Vec<Uuid>> {
        let committee = Committee {
            members: committee_data
                .members
                .iter()
                .map(|m| MemberData {
                    role: m.role.clone(),
                    take_key: m.take_key,
                    dispute_key: m.dispute_key,
                })
                .collect(),
            take_aggregated_key: aggregated_keys.take,
            dispute_aggregated_key: aggregated_keys.dispute,
            packet_size: 10,
            stream_denomination,
        };

        let committee_id = committee_data.committee_uuid();

        info!("Setting up BitVMX committee {committee_id}");

        debug!("Committee details: {committee:?}");

        debug!("Sending SetVar(ADVANCE_FUNDS_INPUT) to BitVMX");
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetVar(
                committee_id,
                ADVANCE_FUNDS_INPUT.to_string(),
                VariableTypes::Utxo(advance_funds_utxo),
            ),
        )
        .context("Failed to send SetVar(ADVANCE_FUNDS_INPUT) to BitVMX")?;

        debug!("Sending SetFundingUtxo to BitVMX");
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetFundingUtxo(my_speedup_funding_utxo),
        )
        .context("Failed to send SetFundingUtxo to BitVMX")?;

        let committee_json = serde_json::to_string(&committee)
            .context("Failed to serialize Committee for BitVMX")?;

        debug!("Sending SetVar(Committee) to BitVMX");
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetVar(
                committee_id,
                Committee::name().to_string(),
                VariableTypes::String(committee_json),
            ),
        )
        .context("Failed to send SetVar(Committee) to BitVMX")?;

        let mut protocol_ids = vec![];
        debug!("Members to setup: {:?}", committee_data.members);
        for member in &committee_data.members {
            let protocol_id = committee_data.get_dispute_core_pid_for_key(&member.take_key)?;

            protocol_ids.push(protocol_id);

            debug!("Setting up dispute core protocol {protocol_id}");

            let dispute_core_data = &DisputeCoreData {
                committee_id,
                member_index: member.committee_idx,
                funding_utxo: member.funding_utxo.clone(),
            };
            let dispute_core_json = serde_json::to_string(dispute_core_data)
                .context("Failed to serialize DisputeCoreData for BitVMX")?;
            debug!("Sending SetVar(DisputeCoreData) to BitVMX: pid={protocol_id}");
            send_bitvmx_msg(
                self.broker_client.as_ref(),
                IncomingBitVMXApiMessages::SetVar(
                    protocol_id,
                    DisputeCoreData::name().to_string(),
                    VariableTypes::String(dispute_core_json),
                ),
            )
            .context("Failed to send SetVar(DisputeCoreData) to BitVMX")?;

            debug!(
                "Sending Setup(DisputeCoreData) to BitVMX: pid={protocol_id}, program_type={PROGRAM_TYPE_DISPUTE_CORE}"
            );
            send_bitvmx_msg(
                self.broker_client.as_ref(),
                IncomingBitVMXApiMessages::Setup(
                    protocol_id,
                    PROGRAM_TYPE_DISPUTE_CORE.to_string(),
                    p2p_addresses.to_owned(),
                    NO_LEADER_IDX,
                ),
            )
            .context("Failed to send Setup(DisputeCoreData) to BitVMX")?;
        }
        debug!("DisputeCoreSetup completed");
        Ok(protocol_ids)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::rc::Rc;

    use alloy_primitives::{Bytes, U256};
    use bitcoin::hashes::Hash;
    use bitcoin::{Amount, PublicKey, ScriptBuf, Txid, WPubkeyHash};
    use common::msg_broker::bitvmx_types::{
        Committee, CommsAddress, DisputeCoreData, IncomingBitVMXApiMessages,
        OutgoingBitVMXApiMessages, OutputType, ParticipantRole, Utxo,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::types::{Address, CommitteeId};
    use primitive_types::H160;
    use uuid::Uuid;

    use super::*;
    use crate::flows::committee::common::CommitteeData;
    use crate::types::MemberOfCommittee;

    // Helper to create a test public key
    fn test_public_key(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 33];
        bytes[0] = 0x02; // compressed
        bytes[1] = seed;
        bytes[2..].fill(seed);
        // This is not a valid secp256k1 point, but sufficient for testing structure
        PublicKey::from_slice(&bytes).unwrap_or_else(|_| {
            // Fallback to a known valid key if parsing fails
            const COMPRESSED_G: [u8; 33] = [
                0x02, 0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce,
                0x87, 0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81,
                0x5b, 0x16, 0xf8, 0x17, 0x98,
            ];
            PublicKey::from_slice(&COMPRESSED_G).expect("valid compressed pubkey")
        })
    }

    // Helper to create a test PartialUtxo
    fn to_u8(index: usize) -> u8 {
        u8::try_from(index).expect("test index must fit in u8")
    }

    fn to_u16(index: usize) -> u16 {
        u16::try_from(index).expect("test index must fit in u16")
    }

    fn to_u32(index: usize) -> u32 {
        u32::try_from(index).expect("test index must fit in u32")
    }

    fn to_u8_from_u32(index: u32) -> u8 {
        u8::try_from(index).expect("test index must fit in u8")
    }

    // Helper to create a test PartialUtxo
    fn test_partial_utxo(index: u32) -> PartialUtxo {
        let mut hash_bytes = [to_u8_from_u32(index); 32];
        hash_bytes[0] = 0; // Ensure it's a valid hash
        let hash = bitcoin::hashes::sha256d::Hash::from_slice(&hash_bytes).expect("valid hash");
        let txid = Txid::from_raw_hash(hash);
        let amount = Amount::from_sat(1000 + u64::from(index));
        let wpkh = WPubkeyHash::from_slice(&[to_u8_from_u32(index); 20]).expect("valid wpkh");
        let script = ScriptBuf::new_p2wpkh(&wpkh);
        let output_type = OutputType::SegwitPublicKey {
            value: amount,
            script_pubkey: script,
            public_key: test_public_key(to_u8_from_u32(index)),
        };
        (txid, index, Some(amount.to_sat()), Some(output_type))
    }

    // Helper to create a test Utxo
    fn test_utxo(index: u32) -> Utxo {
        let mut hash_bytes = [to_u8_from_u32(index); 32];
        hash_bytes[0] = 0; // Ensure it's a valid hash
        let hash = bitcoin::hashes::sha256d::Hash::from_slice(&hash_bytes).expect("valid hash");
        let txid = Txid::from_raw_hash(hash);
        Utxo {
            txid,
            vout: index,
            amount: 1000 + u64::from(index),
            pub_key: test_public_key(to_u8_from_u32(index)),
        }
    }

    // Helper to create a test CommsAddress
    fn test_comms_address(index: usize) -> CommsAddress {
        let socket_addr = SocketAddr::from(([127, 0, 0, 1], 8000 + to_u16(index)));
        CommsAddress { address: socket_addr, pubkey_hash: format!("{index:064x}") }
    }

    // Helper to create a test MemberOfCommittee
    fn test_member(index: usize, role: ParticipantRole) -> MemberOfCommittee {
        let addr_bytes = [to_u8(index); 20];
        let h160 = H160::from(addr_bytes);
        MemberOfCommittee {
            address: Address::from(h160),
            role,
            take_key: test_public_key(to_u8(index * 2)),
            dispute_key: test_public_key(to_u8(index * 2 + 1)),
            funding_utxo: test_partial_utxo(to_u32(index)),
            committee_idx: index,
        }
    }

    // Helper to create test CommitteeData
    fn test_committee_data(members: Vec<MemberOfCommittee>) -> CommitteeData {
        let uuid = Uuid::new_v4();
        let committee_id = CommitteeId::from(uuid.as_u128());
        let default_addr: alloy_primitives::Address = [0u8; 20].into();
        CommitteeData {
            committee_id,
            committee:
                union_contracts::bindings::committee_registry::CommitteeRegistry::Committee {
                    aggregatedKey: Bytes::default(),
                    members: vec![],
                    leaderAddress: default_addr,
                    operatorTakeIndex: U256::from(0),
                    createdAt: U256::from(0),
                    missingData: 0,
                    missingCommunicationData: 0,
                    isPending: false,
                    streamId: 0,
                    fundingUTXOs: vec![],
                },
            members,
        }
    }

    #[test]
    fn test_new_creates_instance() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let _setup = DisputeCoreSetup::new(mock_broker);
        // Just verify it doesn't panic by constructing the instance.
    }

    #[test]
    fn test_setup_sends_advance_funds_input_first() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        let call_order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // Track SetVar calls
        let order_clone = call_order.clone();
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                if let IncomingBitVMXApiMessages::SetVar(_, var_name, _) = msg
                    && var_name == ADVANCE_FUNDS_INPUT
                {
                    order_clone.lock().unwrap().push("ADVANCE_FUNDS_INPUT".to_string());
                    return true;
                }
                false
            })
            .times(1)
            .returning(|_| Ok(true));

        // SetFundingUtxo
        let order_clone = call_order.clone();
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                if matches!(msg, IncomingBitVMXApiMessages::SetFundingUtxo(_)) {
                    order_clone.lock().unwrap().push("SetFundingUtxo".to_string());
                    return true;
                }
                false
            })
            .times(1)
            .returning(|_| Ok(true));

        // Committee SetVar
        let order_clone = call_order.clone();
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                if let IncomingBitVMXApiMessages::SetVar(_, var_name, _) = msg
                    && *var_name == Committee::name()
                {
                    order_clone.lock().unwrap().push("Committee".to_string());
                    return true;
                }
                false
            })
            .times(1)
            .returning(|_| Ok(true));

        // DisputeCoreData SetVar and Setup for each member (2 members * 2 messages = 4)
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, VariableTypes::String(_))
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(4) // 2 members * (SetVar + Setup) = 4
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        let result = setup.setup(
            &committee_data,
            &p2p_addresses,
            AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
            my_speedup_funding_utxo,
            stream_denomination,
            advance_funds_utxo,
        );

        assert!(result.is_ok());
        let protocol_ids = result.unwrap();
        assert_eq!(protocol_ids.len(), 2); // One per member

        // Verify call order: ADVANCE_FUNDS_INPUT should be first
        let order = call_order.lock().unwrap();
        assert_eq!(order[0], "ADVANCE_FUNDS_INPUT");
    }

    #[test]
    fn test_setup_creates_protocol_id_per_member() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(9) // 3 global + 3 members * (SetVar + Setup) = 3 + 6 = 9
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
            test_member(2, ParticipantRole::Prover),
        ];
        let committee_data = test_committee_data(members);
        let p2p_addresses =
            vec![test_comms_address(0), test_comms_address(1), test_comms_address(2)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        let result = setup.setup(
            &committee_data,
            &p2p_addresses,
            AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
            my_speedup_funding_utxo,
            stream_denomination,
            advance_funds_utxo,
        );

        assert!(result.is_ok());
        let protocol_ids = result.unwrap();
        assert_eq!(protocol_ids.len(), 3); // One per member
    }

    #[test]
    fn test_setup_protocol_ids_are_deterministic() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        // setup() is called twice, so we need 2 * (3 global + 2 members * 2) = 2 * 7 = 14 messages
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(14) // 2 calls * (3 global + 2 members * 2) = 2 * 7 = 14
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        // Call setup twice with same parameters
        let protocol_ids1 = setup
            .setup(
                &committee_data,
                &p2p_addresses,
                AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
                my_speedup_funding_utxo.clone(),
                stream_denomination,
                advance_funds_utxo.clone(),
            )
            .unwrap();

        let protocol_ids2 = setup
            .setup(
                &committee_data,
                &p2p_addresses,
                AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
                my_speedup_funding_utxo,
                stream_denomination,
                advance_funds_utxo,
            )
            .unwrap();

        // Protocol IDs should be deterministic
        assert_eq!(protocol_ids1, protocol_ids2);
    }

    #[test]
    fn test_setup_different_members_different_protocol_ids() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(7) // 3 global + 2 members * (SetVar + Setup) = 3 + 4 = 7
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        let protocol_ids = setup
            .setup(
                &committee_data,
                &p2p_addresses,
                AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
                my_speedup_funding_utxo,
                stream_denomination,
                advance_funds_utxo,
            )
            .unwrap();

        // Different members should have different protocol IDs
        assert_eq!(protocol_ids.len(), 2);
        assert_ne!(protocol_ids[0], protocol_ids[1]);
    }

    #[test]
    fn test_setup_sends_committee_with_correct_members() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        let committee_data_capture = std::sync::Arc::new(std::sync::Mutex::new(None));

        // Capture Committee SetVar
        let capture_clone = committee_data_capture.clone();
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                if let IncomingBitVMXApiMessages::SetVar(_, var_name, VariableTypes::String(json)) =
                    msg
                    && *var_name == Committee::name()
                {
                    *capture_clone.lock().unwrap() = Some(json.clone());
                    return true;
                }
                false
            })
            .times(1)
            .returning(|_| Ok(true));

        // Other messages: SetVar(ADVANCE_FUNDS_INPUT) + SetFundingUtxo + 2 members * (SetVar(DisputeCoreData) + Setup)
        // = 1 + 1 + (2 * 2) = 6 messages (SetVar(Committee) is already captured above)
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(6) // SetVar(ADVANCE_FUNDS_INPUT)(1) + SetFundingUtxo(1) + 2 members * 2 = 1 + 1 + 4 = 6
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members.clone());
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        let result = setup.setup(
            &committee_data,
            &p2p_addresses,
            AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
            my_speedup_funding_utxo,
            stream_denomination,
            advance_funds_utxo,
        );

        assert!(result.is_ok());

        // Verify committee JSON contains correct member count
        let captured = committee_data_capture.lock().unwrap();
        if let Some(ref json) = *captured {
            let committee: Committee = serde_json::from_str(json).unwrap();
            assert_eq!(committee.members.len(), 2);
            assert_eq!(committee.members[0].role, ParticipantRole::Prover);
            assert_eq!(committee.members[1].role, ParticipantRole::Verifier);
        } else {
            panic!("Committee data was not captured");
        }
    }

    #[test]
    fn test_setup_sends_dispute_core_data_with_correct_member_index() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        let dispute_core_data_captures = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // Capture DisputeCoreData SetVar calls
        let capture_clone = dispute_core_data_captures.clone();
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                if let IncomingBitVMXApiMessages::SetVar(_, var_name, VariableTypes::String(json)) =
                    msg
                    && *var_name == DisputeCoreData::name()
                {
                    if let Ok(data) = serde_json::from_str::<DisputeCoreData>(json) {
                        capture_clone.lock().unwrap().push(data.member_index);
                    }
                    return true;
                }
                false
            })
            .times(2) // One per member
            .returning(|_| Ok(true));

        // Other messages: SetVar(ADVANCE_FUNDS_INPUT) + SetFundingUtxo + SetVar(Committee) + 2 Setup calls
        // = 1 + 1 + 1 + 2 = 5 messages (SetVar(DisputeCoreData) is already captured above)
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(5) // SetVar(ADVANCE_FUNDS_INPUT)(1) + SetFundingUtxo(1) + SetVar(Committee)(1) + Setup(2) = 5
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        let result = setup.setup(
            &committee_data,
            &p2p_addresses,
            AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
            my_speedup_funding_utxo,
            stream_denomination,
            advance_funds_utxo,
        );

        assert!(result.is_ok());

        // Verify member indices are correct
        let captures = dispute_core_data_captures.lock().unwrap();
        assert_eq!(captures.len(), 2);
        assert!(captures.contains(&0));
        assert!(captures.contains(&1));
    }

    #[test]
    fn test_setup_handles_empty_committee() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                )
            })
            .times(3) // Only global messages, no member-specific
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let committee_data = test_committee_data(vec![]);
        let p2p_addresses = vec![];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 100_000;
        let advance_funds_utxo = test_partial_utxo(200);

        let result = setup.setup(
            &committee_data,
            &p2p_addresses,
            AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
            my_speedup_funding_utxo,
            stream_denomination,
            advance_funds_utxo,
        );

        assert!(result.is_ok());
        let protocol_ids = result.unwrap();
        assert_eq!(protocol_ids.len(), 0); // No members, no protocol IDs
    }

    #[test]
    fn test_setup_preserves_stream_denomination() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        let stream_denom_capture = std::sync::Arc::new(std::sync::Mutex::new(None));

        // Capture Committee SetVar to verify stream_denomination
        let capture_clone = stream_denom_capture.clone();
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                if let IncomingBitVMXApiMessages::SetVar(_, var_name, VariableTypes::String(json)) =
                    msg
                    && *var_name == Committee::name()
                {
                    if let Ok(committee) = serde_json::from_str::<Committee>(json) {
                        *capture_clone.lock().unwrap() = Some(committee.stream_denomination);
                    }
                    return true;
                }
                false
            })
            .times(1)
            .returning(|_| Ok(true));

        // Other messages: SetVar(ADVANCE_FUNDS_INPUT) + SetFundingUtxo + SetVar(DisputeCoreData) + Setup
        // = 1 + 1 + 1 + 1 = 4 messages (SetVar(Committee) is already captured above)
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(_, _, _)
                        | IncomingBitVMXApiMessages::SetFundingUtxo(_)
                        | IncomingBitVMXApiMessages::Setup(_, _, _, _)
                )
            })
            .times(4) // SetVar(ADVANCE_FUNDS_INPUT)(1) + SetFundingUtxo(1) + DisputeCoreData SetVar(1) + Setup(1) = 4
            .returning(|_| Ok(true));

        let setup = DisputeCoreSetup::new(Rc::new(mock_broker));

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0)];
        let take_aggr_key = test_public_key(10);
        let dispute_aggr_key = test_public_key(20);
        let my_speedup_funding_utxo = test_utxo(100);
        let stream_denomination = 123_456_789; // Specific value to verify
        let advance_funds_utxo = test_partial_utxo(200);

        let result = setup.setup(
            &committee_data,
            &p2p_addresses,
            AggregatedKeys { take: take_aggr_key, dispute: dispute_aggr_key },
            my_speedup_funding_utxo,
            stream_denomination,
            advance_funds_utxo,
        );

        assert!(result.is_ok());

        // Verify stream_denomination is preserved
        let captured = stream_denom_capture.lock().unwrap();
        assert_eq!(*captured, Some(stream_denomination));
    }
}
