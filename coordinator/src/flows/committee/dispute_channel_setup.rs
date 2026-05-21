use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result};
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::{
    CommsAddress, ForceChallenge, ForceCondition, IncomingBitVMXApiMessages, OP_COSIGN_UTXOS,
    PROGRAM_TYPE_DISPUTE_CHANNEL, PROGRAM_TYPE_DRP, PartialUtxo, ParticipantRole, VariableTypes,
    WT_INIT_CHALLENGE_UTXOS, WtInitChallengeUtxos, dispute_channel_protocol_id,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::msg_broker::config::{ConfigResult, DisputeConfiguration, ForceFailConfiguration};
use hex::decode;
use log::{debug, info};
use uuid::Uuid;

use crate::flows::committee::common::{CommitteeData, send_bitvmx_msg};
use crate::flows::committee::setup_committee_flow::NO_LEADER_IDX;
use crate::types::MemberOfCommittee;

const DRP_TIMELOCK_BLOCKS: u16 = 15;
const UNION_DRP_AUTO_DISPATCH_INPUT: u8 = 2;
const UNION_DRP_TIMELOCK_MULTIPLIER: u16 = 4;
const UNION_DRP_JOURNAL_SIZE_WORDS: u32 = 76 / 4;
const UNION_DRP_ELF_ID_HEX: &str =
    "589837bb0123b9d5854e0807a8b3ed2b15a848c19e2287ac585a31ec93d711b5";
const UNION_DRP_OPERATOR_ID: [u8; 36] = [1; 36];
const UNION_DRP_FLAGS: [u8; 4] = [1, 0, 1, 0];

#[derive(Clone, Copy)]
struct OperatorSetupParams<'a> {
    committee_data: &'a CommitteeData,
    my_index: usize,
    partner_index: usize,
    my_address: &'a CommsAddress,
    partner_address: &'a CommsAddress,
    pair_key: PublicKey,
    dispute_core_data: &'a [DisputeChannelSetupRequest],
    partner_member: &'a MemberOfCommittee,
}

#[derive(Clone, Copy)]
struct WatchtowerSetupParams<'a> {
    committee_data: &'a CommitteeData,
    my_index: usize,
    partner_index: usize,
    my_address: &'a CommsAddress,
    partner_address: &'a CommsAddress,
    pair_key: PublicKey,
    my_op_cosign_utxos: &'a [Option<PartialUtxo>],
    my_claim_gate_stoppers: &'a [Option<WtInitChallengeUtxos>],
    wt_takekey: &'a PublicKey,
}

struct SetupOneInput<'a> {
    committee_data: &'a CommitteeData,
    op_index: usize,
    wt_index: usize,
    operator: &'a CommsAddress,
    watchtower: &'a CommsAddress,
    pair_key: PublicKey,
    wt_stopper: PartialUtxo,
    op_stopper: PartialUtxo,
    op_cosign: PartialUtxo,
    wt_takekey: &'a PublicKey,
}

/// Manages the setup of `DisputeChannel` protocols between operators and watchtowers.
pub(super) struct DisputeChannelSetup<BC: BitVmxBrokerClientApi> {
    broker_client: Rc<BC>,
    drp_program_definition: String,
}

impl<BC: BitVmxBrokerClientApi> DisputeChannelSetup<BC> {
    pub(super) fn new(broker_client: Rc<BC>, drp_program_definition: String) -> Self {
        Self { broker_client, drp_program_definition }
    }

    /// Requests both `DisputeCore` variables (`OP_COSIGN_UTXOS` and `WT_INIT_CHALLENGE_UTXOS`) for a given PID.
    fn request_dispute_core_variables(&self, dispute_core_pid: Uuid) -> Result<()> {
        debug!(
            "Sending GetVar request to BitVMX: pid={dispute_core_pid}, var_name={OP_COSIGN_UTXOS}"
        );
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::GetVar(dispute_core_pid, OP_COSIGN_UTXOS.to_string()),
        )?;
        debug!(
            "Sending GetVar request to BitVMX: pid={dispute_core_pid}, var_name={WT_INIT_CHALLENGE_UTXOS}"
        );
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::GetVar(
                dispute_core_pid,
                WT_INIT_CHALLENGE_UTXOS.to_string(),
            ),
        )?;
        Ok(())
    }

    /// Requests `DisputeCore` variables and creates the corresponding setup request.
    fn request_and_create_setup_request(
        &self,
        dispute_core_pid: Uuid,
        member_index: usize,
    ) -> Result<DisputeChannelSetupRequest> {
        self.request_dispute_core_variables(dispute_core_pid)?;
        Ok(DisputeChannelSetupRequest {
            dispute_core_pid,
            member_index,
            op_cosign_utxos: None,
            wt_init_challenge_utxos: None,
        })
    }

    /// Initiates the `DisputeChannel` setup process.
    /// First requests my own `DisputeCore` data (`OP_COSIGN_UTXOS` and `WT_INIT_CHALLENGE_UTXOS`).
    /// Returns a list of requests for `DisputeCore` data that will be filled later.
    pub(super) fn request_dispute_core_var(
        &self,
        committee_data: &CommitteeData,
        my_index: usize,
    ) -> Result<Vec<DisputeChannelSetupRequest>> {
        info!(
            "Starting DisputeChannel setup for committee {} as member {}",
            *committee_data.committee_id, my_index
        );

        // Validate bounds
        let my_member = committee_data
            .members
            .get(my_index)
            .context("my_index is out of bounds for members array")?;

        let prover = my_member.role == ParticipantRole::Prover;

        // First, request my own DisputeCore data
        let my_dispute_core_pid = committee_data.get_dispute_core_pid_for_index(my_index)?.value();

        info!(
            "Requesting my own DisputeCore data (member {my_index}) with pid {my_dispute_core_pid}"
        );

        // Build requests list (data will be filled as responses arrive)
        // Start with my own request - this ensures requests are sent before adding to list
        let my_request = self.request_and_create_setup_request(my_dispute_core_pid, my_index)?;
        let mut requests = vec![my_request];

        // Then add requests for partners
        for member in &committee_data.members {
            let partner_index = member.committee_idx;

            // Skip myself
            if partner_index == my_index {
                continue;
            }

            // Skip verifiers pair (if I'm not a prover and partner is also not a prover)
            if !prover && member.role != ParticipantRole::Prover {
                continue;
            }

            let dispute_core_pid =
                committee_data.get_dispute_core_pid_for_index(partner_index)?.value();

            info!(
                "Requesting DisputeCore data for member {partner_index} with pid {dispute_core_pid}"
            );

            let partner_request =
                self.request_and_create_setup_request(dispute_core_pid, partner_index)?;
            requests.push(partner_request);
        }

        info!(
            "Requested DisputeCore data for {} provers (my own data requested first)",
            requests.len()
        );

        Ok(requests)
    }

    /// Extracts stoppers data for a specific index from `DisputeCore` data.
    fn get_stoppers_for_index<'a>(
        wt_init_challenge_utxos: &'a [Option<WtInitChallengeUtxos>],
        index: usize,
        context_msg: &str,
    ) -> Result<&'a WtInitChallengeUtxos> {
        let item = wt_init_challenge_utxos.get(index).context(context_msg.to_string())?;
        let none_msg = format!("{context_msg}: value is None");
        item.as_ref().context(none_msg)
    }

    /// Extracts cosign UTXO for a specific index from `DisputeCore` data.
    fn get_cosign_for_index<'a>(
        op_cosign_utxos: &'a [Option<PartialUtxo>],
        index: usize,
        context_msg: &str,
    ) -> Result<&'a PartialUtxo> {
        let item = op_cosign_utxos.get(index).context(context_msg.to_string())?;
        let none_msg = format!("{context_msg}: value is None");
        item.as_ref().context(none_msg)
    }

    /// Completes the `DisputeChannel` setup once all `DisputeCore` data has been received.
    pub(super) fn complete_setup(
        &self,
        committee_data: &CommitteeData,
        my_index: usize,
        p2p_addresses: &[CommsAddress],
        pairwise_keys: &HashMap<String, PublicKey>,
        dispute_core_data: &[DisputeChannelSetupRequest],
    ) -> Result<Vec<Uuid>> {
        info!(
            "Completing DisputeChannel setup for committee {} as member {}",
            *committee_data.committee_id, my_index
        );

        // Validate bounds
        let my_member = committee_data
            .members
            .get(my_index)
            .context("my_index is out of bounds for members array")?;
        let my_address = p2p_addresses
            .get(my_index)
            .context("my_index is out of bounds for p2p_addresses array")?;

        let prover = my_member.role == ParticipantRole::Prover;

        // Find my DisputeCore data
        let my_core_data = dispute_core_data
            .iter()
            .find(|req| req.member_index == my_index)
            .context("Missing DisputeCore data for my index")?;

        let my_op_cosign_utxos = my_core_data
            .op_cosign_utxos
            .as_ref()
            .context("Missing OP_COSIGN_UTXOS for my index")?;

        let my_claim_gate_stoppers = my_core_data
            .wt_init_challenge_utxos
            .as_ref()
            .context("Missing WT_INIT_CHALLENGE_UTXOS for my index")?;

        let mut protocol_ids = vec![];

        // Iterate over partners
        for (partner_index, partner_member) in committee_data.members.iter().enumerate() {
            if partner_index == my_index {
                // Skip myself
                continue;
            }

            if !prover && partner_member.role != ParticipantRole::Prover {
                // Skip verifiers pair
                continue;
            }

            let partner_address = p2p_addresses.get(partner_index).context(format!(
                "partner_index {partner_index} is out of bounds for p2p_addresses array"
            ))?;

            let key = serde_json::to_string(partner_address)
                .context("Serialize CommsAddress for pairwise_keys lookup")?;
            let pair_key = pairwise_keys.get(&key).copied().context(format!(
                "Pairwise key missing for partner address at index {partner_index}"
            ))?;

            // If I'm an operator, set up DisputeChannel where I'm the operator and my partner is the watchtower
            if prover {
                let protocol_id = self.setup_channel_as_operator(OperatorSetupParams {
                    committee_data,
                    my_index,
                    partner_index,
                    my_address,
                    partner_address,
                    pair_key,
                    dispute_core_data,
                    partner_member,
                })?;
                protocol_ids.push(protocol_id);
            }

            // If my partner is an operator, set up DisputeChannel where they are the operator and I'm the watchtower
            if partner_member.role == ParticipantRole::Prover {
                let protocol_id = self.setup_channel_as_watchtower(WatchtowerSetupParams {
                    committee_data,
                    my_index,
                    partner_index,
                    my_address,
                    partner_address,
                    pair_key,
                    my_op_cosign_utxos,
                    my_claim_gate_stoppers,
                    wt_takekey: &my_member.take_key,
                })?;
                protocol_ids.push(protocol_id);
            }
        }

        info!("DisputeChannel setup completed with {} protocols", protocol_ids.len());

        Ok(protocol_ids)
    }

    /// Sets up a `DisputeChannel` where I am the operator and my partner is the watchtower.
    fn setup_channel_as_operator(&self, p: OperatorSetupParams<'_>) -> Result<Uuid> {
        info!("Setting up DisputeChannel between OP {} and WT {}", p.my_index, p.partner_index);

        // Find partner's DisputeCore data
        let partner_core_data =
            p.dispute_core_data.iter().find(|req| req.member_index == p.partner_index).context(
                format!("Missing DisputeCore data for partner at index {}", p.partner_index),
            )?;

        let partner_claim_gate_stoppers =
            partner_core_data.wt_init_challenge_utxos.as_ref().context(format!(
                "Missing WT_INIT_CHALLENGE_UTXOS for partner at index {}",
                p.partner_index
            ))?;

        let partner_op_cosign_utxos = partner_core_data
            .op_cosign_utxos
            .as_ref()
            .context(format!("Missing OP_COSIGN_UTXOS for partner at index {}", p.partner_index))?;

        let partner_stoppers = Self::get_stoppers_for_index(
            partner_claim_gate_stoppers,
            p.my_index,
            &format!("Missing partner stoppers for my index {} in partner data", p.my_index),
        )?;

        let partner_cosign = Self::get_cosign_for_index(
            partner_op_cosign_utxos,
            p.my_index,
            &format!("Missing partner cosign for my index {} in partner data", p.my_index),
        )?;

        let protocol_id = self.setup_one(SetupOneInput {
            committee_data: p.committee_data,
            op_index: p.my_index,
            wt_index: p.partner_index,
            operator: p.my_address,
            watchtower: p.partner_address,
            pair_key: p.pair_key,
            wt_stopper: partner_stoppers.wt_stopper.clone(),
            op_stopper: partner_stoppers.op_stopper.clone(),
            op_cosign: partner_cosign.clone(),
            wt_takekey: &p.partner_member.take_key,
        })?;

        Ok(protocol_id)
    }

    /// Sets up a `DisputeChannel` where my partner is the operator and I am the watchtower.
    fn setup_channel_as_watchtower(&self, p: WatchtowerSetupParams<'_>) -> Result<Uuid> {
        info!("Setting up DisputeChannel between OP {} and WT {}", p.partner_index, p.my_index);

        let my_stoppers: &WtInitChallengeUtxos = Self::get_stoppers_for_index(
            p.my_claim_gate_stoppers,
            p.partner_index,
            &format!("Missing my stoppers for partner index {} in my data", p.partner_index),
        )?;

        let my_cosign = Self::get_cosign_for_index(
            p.my_op_cosign_utxos,
            p.partner_index,
            &format!("Missing my cosign for partner index {} in my data", p.partner_index),
        )?;

        let protocol_id = self.setup_one(SetupOneInput {
            committee_data: p.committee_data,
            op_index: p.partner_index,
            wt_index: p.my_index,
            operator: p.partner_address,
            watchtower: p.my_address,
            pair_key: p.pair_key,
            wt_stopper: my_stoppers.wt_stopper.clone(),
            op_stopper: my_stoppers.op_stopper.clone(),
            op_cosign: my_cosign.clone(),
            wt_takekey: p.wt_takekey,
        })?;

        Ok(protocol_id)
    }

    fn setup_one(&self, i: SetupOneInput<'_>) -> Result<Uuid> {
        let committee_uuid = i.committee_data.committee_uuid();
        let drp_id = dispute_channel_protocol_id(committee_uuid, i.op_index, i.wt_index).value();
        let dispute_core_pid = i.committee_data.get_dispute_core_pid_for_key(i.wt_takekey).value();
        let participants: Vec<CommsAddress> = vec![i.operator.clone(), i.watchtower.clone()];

        info!(
            "Setting up {} PID {} between OP {} and WT {}",
            PROGRAM_TYPE_DISPUTE_CHANNEL, drp_id, i.op_index, i.wt_index,
        );

        let dispute_config = ForceFailConfiguration {
            prover_force_second_nary: false,
            fail_input_tx: None,
            main: ConfigResult {
                fail_config_prover: None,
                fail_config_verifier: None,
                force_challenge: ForceChallenge::No,
                force_condition: ForceCondition::Always,
            },
            read: ConfigResult::default(),
        };

        self.set_union_verifier_inputs(drp_id)?;

        let dispute_configuration = DisputeConfiguration {
            id: drp_id,
            operators_aggregated_pub: i.pair_key,
            protocol_connection: (i.op_cosign, 1),
            prover_actions: vec![(i.op_stopper, vec![1])], // Consume leaf 1
            prover_enablers: vec![],
            verifier_actions: vec![(i.wt_stopper, vec![1])], // Consume leaf 1
            verifier_enablers: vec![],
            timelock_blocks: DRP_TIMELOCK_BLOCKS.saturating_mul(UNION_DRP_TIMELOCK_MULTIPLIER),
            program_definition: self.drp_program_definition.clone(),
            fail_force_config: Some(dispute_config),
            notify_protocol: vec![("dispute_core".to_string(), dispute_core_pid)],
            auto_dispatch_input: Some(UNION_DRP_AUTO_DISPATCH_INPUT),
        };

        debug!(
            "Sending SetVar request to BitVMX: pid={drp_id}, var_name={}",
            DisputeConfiguration::NAME
        );
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetVar(
                drp_id,
                DisputeConfiguration::NAME.to_string(),
                VariableTypes::String(serde_json::to_string(&dispute_configuration)?),
            ),
        )?;

        debug!(
            "Sending Setup request to BitVMX: pid={drp_id}, program_type={PROGRAM_TYPE_DRP}, participants={}",
            participants.len()
        );
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::Setup(
                drp_id,
                PROGRAM_TYPE_DRP.to_string(),
                participants,
                NO_LEADER_IDX,
            ),
        )?;

        Ok(drp_id)
    }

    fn set_union_verifier_inputs(&self, drp_id: Uuid) -> Result<()> {
        // Match the union verifier setup used by bitvmx-client examples/union.
        self.set_program_input(drp_id, 0, UNION_DRP_JOURNAL_SIZE_WORDS.to_le_bytes().to_vec())?;
        self.set_program_input(
            drp_id,
            1,
            decode(UNION_DRP_ELF_ID_HEX).context("Invalid union verifier ELF id hex")?,
        )?;
        self.set_program_input(drp_id, 3, UNION_DRP_OPERATOR_ID.to_vec())?;
        self.set_program_input(drp_id, 6, UNION_DRP_FLAGS.to_vec())?;
        Ok(())
    }

    fn set_program_input(&self, drp_id: Uuid, input_index: u32, input_data: Vec<u8>) -> Result<()> {
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetVar(
                drp_id,
                format!("program_input_{input_index}"),
                VariableTypes::Input(input_data),
            ),
        )
    }
}

/// Represents a pending `DisputeChannel` setup request.
/// Stores the `DisputeCore` data needed for setup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DisputeChannelSetupRequest {
    pub dispute_core_pid: Uuid,
    pub member_index: usize,
    pub op_cosign_utxos: Option<Vec<Option<PartialUtxo>>>,
    pub wt_init_challenge_utxos: Option<Vec<Option<WtInitChallengeUtxos>>>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::rc::Rc;

    use alloy_primitives::U256;
    use bitcoin::hashes::Hash;
    use bitcoin::{Amount, PublicKey, ScriptBuf, Txid, WPubkeyHash};
    use common::msg_broker::bitvmx_types::{
        CommsAddress, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, OutputType,
        ParticipantRole, WtInitChallengeUtxos,
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

    fn to_u8(index: usize) -> u8 {
        u8::try_from(index).expect("test index must fit in u8")
    }

    fn to_u8_from_u32(index: u32) -> u8 {
        u8::try_from(index).expect("test index must fit in u8")
    }

    fn to_u16(index: usize) -> u16 {
        u16::try_from(index).expect("test index must fit in u16")
    }

    fn to_u32(index: usize) -> u32 {
        u32::try_from(index).expect("test index must fit in u32")
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
            value: amount.into(),
            script_pubkey: script,
            public_key: test_public_key(to_u8_from_u32(index)),
        };
        (txid, index, Some(amount.to_sat()), Some(output_type))
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
                    aggregatedKey: alloy_primitives::Bytes::default(),
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

    // Helper to create WtInitChallengeUtxos
    fn test_wt_init_challenge_utxos(index: usize) -> WtInitChallengeUtxos {
        WtInitChallengeUtxos {
            wt_stopper: test_partial_utxo(to_u32(index * 10)),
            op_stopper: test_partial_utxo(to_u32(index * 10 + 1)),
        }
    }

    #[test]
    fn test_new_creates_instance() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let _setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());
        // Just verify it doesn't panic by constructing the instance.
    }

    #[test]
    fn test_request_dispute_core_var_validates_bounds() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members = vec![
            test_member(0, ParticipantRole::Prover),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);

        // Test out of bounds index
        let result = setup.request_dispute_core_var(&committee_data, 10);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));
    }

    #[test]
    fn test_request_dispute_core_var_includes_my_request_first() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        // Expect GetVar calls for my dispute core PID
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(msg, IncomingBitVMXApiMessages::GetVar(_, _))
            })
            .times(2) // OP_COSIGN_UTXOS and WT_INIT_CHALLENGE_UTXOS
            .returning(|_| Ok(true));

        let setup = DisputeChannelSetup::new(Rc::new(mock_broker), "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);

        let requests = setup.request_dispute_core_var(&committee_data, 0).unwrap();

        // My request should be first
        assert_eq!(requests[0].member_index, 0);
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn test_request_dispute_core_var_filters_verifier_pairs() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(msg, IncomingBitVMXApiMessages::GetVar(_, _))
            })
            .times(2) // Only my own (verifier doesn't request other verifiers)
            .returning(|_| Ok(true));

        let setup = DisputeChannelSetup::new(Rc::new(mock_broker), "test.yaml".to_string());

        let members = vec![
            test_member(0, ParticipantRole::Verifier),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);

        let requests = setup.request_dispute_core_var(&committee_data, 0).unwrap();

        // Should only have my request, not the other verifier
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].member_index, 0);
    }

    #[test]
    fn test_request_dispute_core_var_includes_prover_partners() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        // Expect GetVar for my PID (2 calls) + prover partner (2 calls)
        mock_broker
            .expect_send()
            .withf(|msg: &IncomingBitVMXApiMessages| {
                matches!(msg, IncomingBitVMXApiMessages::GetVar(_, _))
            })
            .times(4)
            .returning(|_| Ok(true));

        let setup = DisputeChannelSetup::new(Rc::new(mock_broker), "test.yaml".to_string());

        let members =
            vec![test_member(0, ParticipantRole::Prover), test_member(1, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);

        let requests = setup.request_dispute_core_var(&committee_data, 0).unwrap();

        // Should have my request and prover partner request
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].member_index, 0); // My request first
        assert_eq!(requests[1].member_index, 1); // Partner request
    }

    #[test]
    fn test_complete_setup_validates_bounds() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0)];
        let pairwise_keys = HashMap::new();
        let dispute_core_data = vec![];

        // Test out of bounds my_index
        let result = setup.complete_setup(
            &committee_data,
            10,
            &p2p_addresses,
            &pairwise_keys,
            &dispute_core_data,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));

        // Test out of bounds p2p_addresses
        let result =
            setup.complete_setup(&committee_data, 0, &[], &pairwise_keys, &dispute_core_data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));
    }

    #[test]
    fn test_complete_setup_requires_my_dispute_core_data() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0)];
        let pairwise_keys = HashMap::new();
        let dispute_core_data = vec![]; // Empty - missing my data

        let result = setup.complete_setup(
            &committee_data,
            0,
            &p2p_addresses,
            &pairwise_keys,
            &dispute_core_data,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing DisputeCore data"));
    }

    #[test]
    fn test_complete_setup_requires_op_cosign_utxos() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0)];
        let pairwise_keys = HashMap::new();
        let dispute_core_data = vec![DisputeChannelSetupRequest {
            dispute_core_pid: Uuid::new_v4(),
            member_index: 0,
            op_cosign_utxos: None, // Missing
            wt_init_challenge_utxos: None,
        }];

        let result = setup.complete_setup(
            &committee_data,
            0,
            &p2p_addresses,
            &pairwise_keys,
            &dispute_core_data,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing OP_COSIGN_UTXOS"));
    }

    #[test]
    fn test_complete_setup_requires_wt_init_challenge_utxos() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0)];
        let pairwise_keys = HashMap::new();
        let dispute_core_data = vec![DisputeChannelSetupRequest {
            dispute_core_pid: Uuid::new_v4(),
            member_index: 0,
            op_cosign_utxos: Some(vec![]),
            wt_init_challenge_utxos: None, // Missing
        }];

        let result = setup.complete_setup(
            &committee_data,
            0,
            &p2p_addresses,
            &pairwise_keys,
            &dispute_core_data,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing WT_INIT_CHALLENGE_UTXOS"));
    }

    #[test]
    fn test_complete_setup_requires_pairwise_key() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members =
            vec![test_member(0, ParticipantRole::Prover), test_member(1, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let pairwise_keys = HashMap::new(); // Missing pairwise key
        let dispute_core_data = vec![DisputeChannelSetupRequest {
            dispute_core_pid: Uuid::new_v4(),
            member_index: 0,
            op_cosign_utxos: Some(vec![Some(test_partial_utxo(0))]),
            wt_init_challenge_utxos: Some(vec![Some(test_wt_init_challenge_utxos(0))]),
        }];

        let result = setup.complete_setup(
            &committee_data,
            0,
            &p2p_addresses,
            &pairwise_keys,
            &dispute_core_data,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Pairwise key missing"));
    }

    #[test]
    fn test_complete_setup_skips_verifier_pairs() {
        let mock_broker = Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let setup = DisputeChannelSetup::new(mock_broker, "test.yaml".to_string());

        let members = vec![
            test_member(0, ParticipantRole::Verifier),
            test_member(1, ParticipantRole::Verifier),
        ];
        let committee_data = test_committee_data(members);
        let p2p_addresses = vec![test_comms_address(0), test_comms_address(1)];
        let pairwise_keys = HashMap::new();
        let dispute_core_data = vec![DisputeChannelSetupRequest {
            dispute_core_pid: Uuid::new_v4(),
            member_index: 0,
            op_cosign_utxos: Some(vec![Some(test_partial_utxo(0))]),
            wt_init_challenge_utxos: Some(vec![Some(test_wt_init_challenge_utxos(0))]),
        }];

        let result = setup.complete_setup(
            &committee_data,
            0,
            &p2p_addresses,
            &pairwise_keys,
            &dispute_core_data,
        );
        // Should succeed but return empty protocol_ids (no channels between verifiers)
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_get_stoppers_for_index_validates_bounds() {
        let wt_init_challenge_utxos = vec![Some(test_wt_init_challenge_utxos(0))];

        let result = DisputeChannelSetup::<
            MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
        >::get_stoppers_for_index(&wt_init_challenge_utxos, 10, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("test"));
    }

    #[test]
    fn test_get_stoppers_for_index_handles_none() {
        let wt_init_challenge_utxos = vec![None];

        let result = DisputeChannelSetup::<
            MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
        >::get_stoppers_for_index(&wt_init_challenge_utxos, 0, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("None"));
    }

    #[test]
    fn test_get_cosign_for_index_validates_bounds() {
        let op_cosign_utxos = vec![Some(test_partial_utxo(0))];

        let result = DisputeChannelSetup::<
            MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
        >::get_cosign_for_index(&op_cosign_utxos, 10, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("test"));
    }

    #[test]
    fn test_get_cosign_for_index_handles_none() {
        let op_cosign_utxos = vec![None];

        let result = DisputeChannelSetup::<
            MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
        >::get_cosign_for_index(&op_cosign_utxos, 0, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("None"));
    }

    #[test]
    fn test_setup_one_sends_correct_messages() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        let setup_sequence = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let seq_clone = setup_sequence.clone();
        // setup_one seeds union-verifier inputs, then sends dispute configuration and setup.
        mock_broker
            .expect_send()
            .withf(move |msg: &IncomingBitVMXApiMessages| {
                seq_clone.lock().unwrap().push(format!("{msg:?}"));
                true
            })
            .times(6)
            .returning(|_| Ok(true));

        let setup = DisputeChannelSetup::new(Rc::new(mock_broker), "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let operator = test_comms_address(0);
        let watchtower = test_comms_address(1);
        let pair_key = test_public_key(42);
        let wt_stopper = test_partial_utxo(1);
        let op_stopper = test_partial_utxo(2);
        let op_cosign = test_partial_utxo(3);
        let wt_takekey = test_public_key(10);

        let result = setup.setup_one(SetupOneInput {
            committee_data: &committee_data,
            op_index: 0,
            wt_index: 1,
            operator: &operator,
            watchtower: &watchtower,
            pair_key,
            wt_stopper,
            op_stopper,
            op_cosign,
            wt_takekey: &wt_takekey,
        });

        assert!(result.is_ok());
        let protocol_id = result.unwrap();
        assert!(!protocol_id.is_nil());

        // Verify all verifier-input and setup messages were sent.
        let sequence = setup_sequence.lock().unwrap();
        assert_eq!(sequence.len(), 6);
    }

    #[test]
    fn test_setup_one_uses_deterministic_protocol_id() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        // setup_one sends 6 messages per call, so 12 total for 2 calls.
        mock_broker.expect_send().times(12).returning(|_| Ok(true));

        let setup = DisputeChannelSetup::new(Rc::new(mock_broker), "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let operator = test_comms_address(0);
        let watchtower = test_comms_address(1);
        let pair_key = test_public_key(42);
        let wt_stopper = test_partial_utxo(1);
        let op_stopper = test_partial_utxo(2);
        let op_cosign = test_partial_utxo(3);
        let wt_takekey = test_public_key(10);

        // Call setup_one twice with same parameters
        let id1 = setup
            .setup_one(SetupOneInput {
                committee_data: &committee_data,
                op_index: 0,
                wt_index: 1,
                operator: &operator,
                watchtower: &watchtower,
                pair_key,
                wt_stopper: wt_stopper.clone(),
                op_stopper: op_stopper.clone(),
                op_cosign: op_cosign.clone(),
                wt_takekey: &wt_takekey,
            })
            .unwrap();

        let id2 = setup
            .setup_one(SetupOneInput {
                committee_data: &committee_data,
                op_index: 0,
                wt_index: 1,
                operator: &operator,
                watchtower: &watchtower,
                pair_key,
                wt_stopper,
                op_stopper,
                op_cosign,
                wt_takekey: &wt_takekey,
            })
            .unwrap();

        // Protocol IDs should be deterministic
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_setup_one_different_indices_different_ids() {
        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        // setup_one sends 6 messages per call, so 12 total for 2 calls.
        mock_broker.expect_send().times(12).returning(|_| Ok(true));

        let setup = DisputeChannelSetup::new(Rc::new(mock_broker), "test.yaml".to_string());

        let members = vec![test_member(0, ParticipantRole::Prover)];
        let committee_data = test_committee_data(members);
        let operator = test_comms_address(0);
        let watchtower = test_comms_address(1);
        let pair_key = test_public_key(42);
        let wt_stopper = test_partial_utxo(1);
        let op_stopper = test_partial_utxo(2);
        let op_cosign = test_partial_utxo(3);
        let wt_takekey = test_public_key(10);

        let id_0_1 = setup
            .setup_one(SetupOneInput {
                committee_data: &committee_data,
                op_index: 0,
                wt_index: 1,
                operator: &operator,
                watchtower: &watchtower,
                pair_key,
                wt_stopper: wt_stopper.clone(),
                op_stopper: op_stopper.clone(),
                op_cosign: op_cosign.clone(),
                wt_takekey: &wt_takekey,
            })
            .unwrap();

        let id_1_0 = setup
            .setup_one(SetupOneInput {
                committee_data: &committee_data,
                op_index: 1,
                wt_index: 0,
                operator: &operator,
                watchtower: &watchtower,
                pair_key,
                wt_stopper,
                op_stopper,
                op_cosign,
                wt_takekey: &wt_takekey,
            })
            .unwrap();

        // Different indices should produce different IDs
        assert_ne!(id_0_1, id_1_0);
    }
}
