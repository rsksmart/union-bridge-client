use anyhow::{Context, Result};
use bitcoin::{PublicKey, Txid};
use common::msg_broker::bitvmx_types::{
    Committee, DisputeCoreData, IncomingBitVMXApiMessages, MemberData, P2PAddress, PartialUtxo,
    ParticipantRole, VariableTypes,
};
use log::info;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;
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
        my_id: String,
        committee_id_client: CommitteeId,
        members: Vec<MemberOfCommittee>,
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

        self.send_set_var(
            committee_id,
            Committee::name(),
            VariableTypes::String(serde_json::to_string(&committee)?),
        )?;

        for (operator_index, member) in members.iter().enumerate() {
            if member.role == ParticipantRole::Prover {
                let pubkey = member.take_key;
                let protocol_id = get_dispute_core_pid(committee_id, &pubkey)?;

                info!("Setting up the DisputeCore protocol handler {protocol_id} for {my_id}");

                self.send_set_var(
                    protocol_id,
                    DisputeCoreData::name(),
                    VariableTypes::String(serde_json::to_string(&DisputeCoreData {
                        committee_id,
                        operator_index,
                        operator_utxo: member.funding_utxo.clone(),
                        operator_take_pubkey: pubkey,
                    })?),
                )?;

                self.send_set_var(
                    protocol_id,
                    MONITORED_OPERATOR_KEY.to_string(),
                    VariableTypes::PubKey(pubkey),
                )?;

                self.send_setup(
                    protocol_id,
                    PROGRAM_TYPE_DISPUTE_CORE.to_string(),
                    Self::get_addresses(&members.clone()),
                )?;
            }
        }

        Ok(())
    }

    fn operator_count(members: &[MemberOfCommittee]) -> Result<u32> {
        Ok(members
            .iter()
            .filter(|m| m.role == ParticipantRole::Prover)
            .count() as u32)
    }

    fn get_addresses(members: &[MemberOfCommittee]) -> Vec<P2PAddress> {
        members.iter().flat_map(|m| m.p2p_addrs.clone()).collect()
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
