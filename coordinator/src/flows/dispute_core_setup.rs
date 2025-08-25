use anyhow::{Context, Result};
use bitcoin::{Amount, PublicKey, ScriptBuf};
use common::msg_broker::bitvmx_types::{
    Committee, DisputeCoreData, IncomingBitVMXApiMessages, MemberData, OutputType, P2PAddress,
    PartialUtxo, ParticipantRole, Utxo, VariableTypes,
};
use log::{debug, info};
use std::rc::Rc;
use uuid::Uuid;

use crate::flows::setup_committee_flow::NO_LEADER_IDX;
use crate::types::MemberOfCommittee;
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::types::CommitteeId;
use sha2::{Digest, Sha256};

const MONITORED_OPERATOR_KEY: &str = "monitored_operator_key";
const PROGRAM_TYPE_DISPUTE_CORE: &str = "dispute_core";

pub struct DisputeCoreSetup<BC: BitVmxBrokerClientApi> {
    broker_client: Rc<BC>,
}

// TODO(iago-3) search for possible panics on this file

impl<BC: BitVmxBrokerClientApi> DisputeCoreSetup<BC> {
    pub fn new(broker_client: Rc<BC>) -> Self {
        Self { broker_client }
    }

    pub fn setup(
        &self,
        committee_id_client: CommitteeId,
        members: Vec<MemberOfCommittee>,
        p2p_addresses: Vec<P2PAddress>,
        take_aggr_key: PublicKey,
        dispute_aggr_key: PublicKey,
    ) -> Result<()> {
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
            operator_count: Self::operator_count(&members)?,
            packet_size: 10,
        };

        let committee_id = Uuid::from_u128(*committee_id_client);

        info!("Setting up the BitVMX Committee {committee_id}");

        debug!("Sending BitVMX Committee {committee:?}");

        let my_funding_utxo = Self::get_my_speedup_funding_utxo(dispute_aggr_key)?;
        self.broker_client.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SetFundingUtxo(my_funding_utxo),
        )?;

        self.send_set_var(
            committee_id,
            Committee::name(),
            VariableTypes::String(serde_json::to_string(&committee)?),
        )?;

        for (operator_index, member) in members.iter().enumerate() {
            if member.role == ParticipantRole::Prover {
                let pubkey = member.take_key;
                let protocol_id = get_dispute_core_pid(committee_id, &pubkey)?;

                info!("Setting up the DisputeCore protocol {protocol_id}");

                let dispute_core_data = &DisputeCoreData {
                    committee_id,
                    operator_index,
                    operator_utxo: member.funding_utxo.clone(),
                    operator_take_pubkey: pubkey,
                };

                debug!("Sending BitVMX DisputeCoreData {dispute_core_data:?}");

                self.send_set_var(
                    protocol_id,
                    DisputeCoreData::name(),
                    VariableTypes::String(serde_json::to_string(dispute_core_data)?),
                )?;

                self.send_set_var(
                    protocol_id,
                    MONITORED_OPERATOR_KEY.to_string(),
                    VariableTypes::PubKey(pubkey),
                )?;

                debug!("Sending BitVMX Setup {p2p_addresses:?}");

                self.send_setup(
                    protocol_id,
                    PROGRAM_TYPE_DISPUTE_CORE.to_string(),
                    p2p_addresses.clone(),
                )?;
            }
        }

        Ok(())
    }

    fn get_my_speedup_funding_utxo(dispute_aggr_key: PublicKey) -> Result<Utxo> {
        // TODO(iago) use contract ones when fixed
        let amount = 10_000_000;
        let vout = 0;
        let txid = "4d5f11a0b73b61cbb2f5e21a09a0f1f0e9dbbdbff85f2a9dbe46e2c3b2e6b5d0".parse()?;

        Ok(Utxo::new(txid, vout, amount, &dispute_aggr_key))
    }

    fn operator_count(members: &[MemberOfCommittee]) -> Result<u32> {
        Ok(members
            .iter()
            .filter(|m| m.role == ParticipantRole::Prover)
            .count() as u32)
    }

    fn send_set_var(
        &self,
        protocol_id: Uuid,
        protocol_name: String,
        var: VariableTypes,
    ) -> Result<()> {
        self.broker_client.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SetVar(protocol_id, protocol_name, var),
        )?;

        Ok(())
    }

    fn send_setup(
        &self,
        protocol_id: Uuid,
        protocol_name: String,
        addresses: Vec<P2PAddress>,
    ) -> Result<()> {
        self.broker_client.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::Setup(protocol_id, protocol_name, addresses, NO_LEADER_IDX),
        )?;

        Ok(())
    }
}

fn get_dispute_core_pid(committee_id: Uuid, pubkey: &PublicKey) -> Result<Uuid> {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(pubkey.to_bytes());
    hasher.update("dispute_core");

    // Get the result as a byte array
    let hash = hasher.finalize();
    let bytes = hash[0..16]
        .try_into()
        .context("UUID slice conversion failed")?;

    Ok(Uuid::from_bytes(bytes))
}
