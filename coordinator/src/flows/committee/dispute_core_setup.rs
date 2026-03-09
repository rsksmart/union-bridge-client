use std::rc::Rc;

use anyhow::{Context, Result};
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::{
    Committee, DisputeCoreData, IncomingBitVMXApiMessages, MemberData, P2PAddress, ParticipantRole,
    Utxo, VariableTypes,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::types::CommitteeId;
use log::{debug, error, info, trace};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::flows::committee::setup_committee_flow::NO_LEADER_IDX;
use crate::types::MemberOfCommittee;

const MONITORED_OPERATOR_KEY: &str = "monitored_operator_key";
const PROGRAM_TYPE_DISPUTE_CORE: &str = "dispute_core";

pub struct DisputeCoreSetup<BC: BitVmxBrokerClientApi> {
    broker_client: Rc<BC>,
}

impl<BC: BitVmxBrokerClientApi> DisputeCoreSetup<BC> {
    pub fn new(broker_client: Rc<BC>) -> Self {
        Self { broker_client }
    }

    pub fn setup(
        &self,
        committee_id_client: &CommitteeId,
        members: &[MemberOfCommittee],
        p2p_addresses: &[P2PAddress],
        take_aggr_key: PublicKey,
        dispute_aggr_key: PublicKey,
        my_speedup_funding_utxo: Utxo,
    ) -> Result<Vec<Uuid>> {
        let committee = Committee {
            members: members
                .iter()
                .map(|m| MemberData {
                    role: m.role.clone(),
                    take_key: m.take_key,
                    dispute_key: m.dispute_key,
                })
                .collect(),
            take_aggregated_key: take_aggr_key,
            dispute_aggregated_key: dispute_aggr_key,
            operator_count: Self::operator_count(members)?,
            packet_size: 10,
        };

        let committee_id = Uuid::from_u128(**committee_id_client);

        info!("Setting up BitVMX committee {committee_id}");

        trace!("Committee details: {committee:?}");

        self.broker_client.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SetFundingUtxo(my_speedup_funding_utxo),
        )?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            committee_id,
            Committee::name(),
            VariableTypes::String(serde_json::to_string(&committee)?),
        ));

        let mut protocol_ids = vec![];

        let provers = members
            .iter()
            .filter(|m| m.role == ParticipantRole::Prover)
            .cloned()
            .collect::<Vec<_>>();

        for prover in provers {
            let pubkey = prover.take_key;
            let protocol_id = get_dispute_core_pid(committee_id, &pubkey)?;

            protocol_ids.push(protocol_id);

            debug!("Setting up dispute core protocol {protocol_id}");

            let dispute_core_data = &DisputeCoreData {
                committee_id,
                operator_index: prover.committee_idx,
                operator_utxo: prover.funding_utxo.clone(),
                operator_take_pubkey: pubkey,
            };

            self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
                protocol_id,
                DisputeCoreData::name(),
                VariableTypes::String(serde_json::to_string(dispute_core_data)?),
            ));

            self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
                protocol_id,
                MONITORED_OPERATOR_KEY.to_string(),
                VariableTypes::PubKey(pubkey),
            ));

            self.send_bitvmx_msg(IncomingBitVMXApiMessages::Setup(
                protocol_id,
                PROGRAM_TYPE_DISPUTE_CORE.to_string(),
                p2p_addresses.to_owned(),
                NO_LEADER_IDX,
            ));
        }

        Ok(protocol_ids)
    }

    fn operator_count(members: &[MemberOfCommittee]) -> Result<u32> {
        u32::try_from(members.iter().filter(|m| m.role == ParticipantRole::Prover).count())
            .context("operator count exceeds u32::MAX")
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) {
        trace!("Sending to BitVMX: {msg:?}");

        let result = self.broker_client.send(BROKER_SERVER_ID, msg);
        if result.is_err() {
            // TODO(Jira) UB-132
            error!("Failed to send msg to BitVMX: {result:?}");
        }
    }
}

fn get_dispute_core_pid(committee_id: Uuid, pubkey: &PublicKey) -> Result<Uuid> {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(pubkey.to_bytes());
    hasher.update("dispute_core");

    // Get the result as a byte array
    let hash = hasher.finalize();
    let bytes = hash[0..16].try_into().context("UUID slice conversion failed")?;

    Ok(Uuid::from_bytes(bytes))
}
