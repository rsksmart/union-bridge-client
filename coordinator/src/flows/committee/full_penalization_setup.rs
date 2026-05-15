use std::rc::Rc;

use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::{
    BitVmxProtocolId, CommsAddress, FullPenalizationData, IncomingBitVMXApiMessages,
    PROGRAM_TYPE_FULL_PENALIZATION, VariableTypes, full_penalization_protocol_id,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use tracing::{debug, info};
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
    ) -> Result<BitVmxProtocolId> {
        let protocol_id = full_penalization_protocol_id(committee_id);

        info!("Setting up the FullPenalization protocol handler {protocol_id} for {my_id}");

        let data = FullPenalizationData { committee_id };

        let payload = serde_json::to_string(&data)
            .context("Failed to serialize FullPenalizationData for BitVMX")?;

        debug!("Sending SetVar(FullPenalizationData) to BitVMX: pid={protocol_id}");
        send_bitvmx_msg(
            self.broker_client.as_ref(),
            IncomingBitVMXApiMessages::SetVar(
                protocol_id.value(),
                FullPenalizationData::name().clone(),
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
                protocol_id.value(),
                PROGRAM_TYPE_FULL_PENALIZATION.to_string(),
                addresses.to_vec(),
                0,
            ),
        )
        .context("Failed to send Setup(FullPenalization) to BitVMX")?;

        Ok(protocol_id)
    }
}
