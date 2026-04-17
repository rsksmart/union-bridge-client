use std::rc::Rc;

use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::{
    CommsAddress, FullPenalizationData, IncomingBitVMXApiMessages, PROGRAM_TYPE_FULL_PENALIZATION,
    VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use log::{debug, info};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::flows::committee::common::send_bitvmx_msg;

pub struct FullPenalizationSetup<BC: BitVmxBrokerClientApi> {
    broker_client: Rc<BC>,
}

impl<BC: BitVmxBrokerClientApi> FullPenalizationSetup<BC> {
    pub fn new(broker_client: Rc<BC>) -> Self {
        Self { broker_client }
    }

    pub fn setup(
        &self,
        committee_id: Uuid,
        my_id: usize,
        addresses: &[CommsAddress],
    ) -> Result<Uuid> {
        let protocol_id = Self::get_full_penalization_pid(committee_id);

        info!("Setting up the FullPenalization protocol handler {} for {}", protocol_id, my_id);

        let data = FullPenalizationData { committee_id };

        let payload = serde_json::to_string(&data)
            .context("Failed to serialize FullPenalizationData for BitVMX")?;

        debug!("Sending SetVar(FullPenalizationData) to BitVMX: pid={protocol_id}");
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetVar(
                protocol_id,
                FullPenalizationData::name().to_string(),
                VariableTypes::String(payload),
            ),
        )
        .context("Failed to send SetVar(FullPenalizationData) to BitVMX")?;

        debug!(
            "Sending Setup(FullPenalization) to BitVMX: pid={protocol_id}, program_type={PROGRAM_TYPE_FULL_PENALIZATION}"
        );
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::Setup(
                protocol_id,
                PROGRAM_TYPE_FULL_PENALIZATION.to_string(),
                addresses.to_vec(),
                0,
            ),
        )
        .context("Failed to send Setup(FullPenalization) to BitVMX")?;

        Ok(protocol_id)
    }

    fn get_full_penalization_pid(committee_id: Uuid) -> Uuid {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update("full_penalization");

        // Get the result as a byte array
        let hash = hasher.finalize();
        return Uuid::from_bytes(hash[0..16].try_into().unwrap());
    }
}
