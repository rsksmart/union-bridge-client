use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::UserRequests;
use anyhow::{Result, anyhow, bail};
use bitcoin::PublicKey;
use common::runtime_sync::RuntimeSync;
use log::{error, info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, P2PAddress,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use transaction_dispatcher::types::ApplyToStreamInput;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowFactoryApi<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi>
{
    fn create_flow(&self, flow_id: Uuid) -> SetupCommitteeFlow<CG, BC>;
}

#[derive(Default, Debug)]
pub struct Context {
    pub user_input: Option<ApplyToStreamInput>,
    pub p2p_address: Option<P2PAddress>,
    pub take_key: Option<PublicKey>,
    pub dispute_key: Option<PublicKey>,
    pub agg_take_key: Option<PublicKey>,
    pub agg_dispute_key: Option<PublicKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Steps {
    NotStarted,
    //
    UserInput,
    BitVmxCommInfo,
    BitVmxTakeKey,
    BitVmxDisputeKey,
    SetupTakeAggregatedKey,
    SetupDisputeAggregatedKey,
    //
    Complete,
}

impl Steps {
    fn next(&self) -> Result<Steps> {
        let next = match self {
            Steps::NotStarted => Steps::UserInput,
            Steps::UserInput => Steps::BitVmxCommInfo,
            Steps::BitVmxCommInfo => Steps::BitVmxTakeKey,
            Steps::BitVmxTakeKey => Steps::BitVmxDisputeKey,
            Steps::BitVmxDisputeKey => Steps::SetupTakeAggregatedKey,
            Steps::SetupTakeAggregatedKey => Steps::SetupDisputeAggregatedKey,
            Steps::SetupDisputeAggregatedKey => Steps::Complete,
            Steps::Complete => {
                bail!("Flow is already complete at {:?}", self)
            }
        };

        Ok(next)
    }
}

enum StepData {
    ApplyToStreamInput(ApplyToStreamInput),
    P2PAddress(P2PAddress),
    PublicKey(PublicKey),
}

impl StepData {
    fn into_user_input(self) -> Result<ApplyToStreamInput> {
        match self {
            StepData::ApplyToStreamInput(input) => Ok(input),
            _ => bail!("Expected ApplyToStreamInput"),
        }
    }

    fn into_p2p_address(self) -> Result<P2PAddress> {
        match self {
            StepData::P2PAddress(addr) => Ok(addr),
            _ => bail!("Expected P2PAddress"),
        }
    }

    fn into_pubkey(self) -> Result<PublicKey> {
        match self {
            StepData::PublicKey(pk) => Ok(pk),
            _ => bail!("Expected PublicKey"),
        }
    }
}

pub(crate) struct State {
    flow_id: Uuid,
    step: Steps,
    ctx: Context,
}

pub(crate) struct SetupCommitteeFlow<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi> {
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    blockchain_view: Rc<RefCell<BlockchainView>>,
    state: State,
}

impl<CG, BC> SetupCommitteeFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn new(contracts: Rc<CG>, rt_sync: RuntimeSync, bitvmx_broker: Rc<BC>, flow_id: Uuid) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            blockchain_view: Rc::new(RefCell::new(BlockchainView::new())),
            state: State {
                flow_id,
                step: Steps::NotStarted,
                ctx: Context::default(),
            },
        }
    }

    fn next_step(&mut self, data: StepData) -> Result<()> {
        self.state.step = self.state.step.next()?;

        match self.state.step {
            Steps::NotStarted => {
                bail!("Flow is in NotStarted state");
            }
            Steps::UserInput => {
                self.process_user_input(data)?;
            }
            Steps::BitVmxCommInfo => {
                self.process_comm_info(data)?;
            }
            Steps::BitVmxTakeKey => {
                self.process_take_key(data)?;
            }
            Steps::BitVmxDisputeKey => {
                self.process_dispute_key(data)?;
            }
            Steps::SetupTakeAggregatedKey => {
                self.process_take_aggregated_key(data)?;
            }
            Steps::SetupDisputeAggregatedKey => {
                self.process_dispute_aggregated_key(data)?;
            }
            Steps::Complete => {
                info!("Setup committee flow complete")
            }
        };

        Ok(())
    }

    fn process_user_input(&mut self, data: StepData) -> Result<()> {
        let input = data.into_user_input()?;
        self.state.ctx.user_input = Some(input.clone());
        self.request_bitvmx_comm_info(&input)?;
        Ok(())
    }

    fn process_comm_info(&mut self, data: StepData) -> Result<()> {
        let addr = data.into_p2p_address()?;
        self.state.ctx.p2p_address = Some(addr.clone());
        self.request_bitvmx_take_pub_key()?;
        Ok(())
    }

    fn process_take_key(&mut self, data: StepData) -> Result<()> {
        let key = data.into_pubkey()?;
        self.state.ctx.take_key = Some(key);
        self.request_bitvmx_dispute_pub_key()?;
        Ok(())
    }

    fn process_dispute_key(&mut self, data: StepData) -> Result<()> {
        let key = data.into_pubkey()?;
        self.state.ctx.dispute_key = Some(key);
        self.setup_take_aggregated_pubkey()?;
        Ok(())
    }

    fn process_take_aggregated_key(&mut self, data: StepData) -> Result<()> {
        let key = data.into_pubkey()?;
        self.state.ctx.agg_take_key = Some(key);
        self.setup_dispute_aggregated_pubkey()?;
        Ok(())
    }

    fn process_dispute_aggregated_key(&mut self, data: StepData) -> Result<()> {
        let key = data.into_pubkey()?;
        self.state.ctx.agg_dispute_key = Some(key);
        Ok(())
    }

    fn request_bitvmx_comm_info(&self, _input: &ApplyToStreamInput) -> Result<()> {
        // TODO how do they know for whom to get comm info?
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo());
        Ok(())
    }

    fn request_bitvmx_take_pub_key(&self) -> Result<()> {
        self.request_bitvmx_member_pub_key()
    }

    fn request_bitvmx_dispute_pub_key(&self) -> Result<()> {
        self.request_bitvmx_member_pub_key()
    }

    fn request_bitvmx_member_pub_key(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetPubKey(
            self.state.flow_id,
            true,
        ));
        Ok(())
    }

    fn setup_take_aggregated_pubkey(&self) -> Result<()> {
        // TODO(ask-Fairgate) how to get these keys? I guess it's for the whole committee
        let mut committee_take_keys = vec![];

        if let Some(my_take_key) = self.state.ctx.take_key {
            committee_take_keys.push(my_take_key);
            self.setup_aggregated_pubkey(committee_take_keys)
        } else {
            bail!("Take key not found in context");
        }
    }

    fn setup_dispute_aggregated_pubkey(&self) -> Result<()> {
        // TODO(ask-Fairgate) how to get these keys? I guess it's for the whole committee
        let mut committee_dispute_keys = vec![];

        if let Some(my_dispute_key) = self.state.ctx.dispute_key {
            committee_dispute_keys.push(my_dispute_key);
            self.setup_aggregated_pubkey(committee_dispute_keys)
        } else {
            bail!("Dispute key not found in context");
        }
    }

    fn setup_aggregated_pubkey(&self, participants_keys: Vec<PublicKey>) -> Result<()> {
        let leader_idx = 0; // TODO not yet implemented
        let p2p_addresses = vec![]; // TODO(ask-Fairgate) CommitteeRegistry.getMemberCommunicationData???

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
            self.state.flow_id,
            p2p_addresses,
            Some(participants_keys),
            leader_idx,
        ));

        Ok(())
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) {
        info!("Sending {msg:?} to BitVMX");

        let result = self.bitvmx_broker.send(BROKER_SERVER_ID, msg);
        if result.is_err() {
            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
            error!("Failed to send msg to BitVMX: {:?}", result);
        }
    }
}

pub(crate) struct SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    flow_factory: FactoryBSF,
    flows: HashMap<Uuid, SetupCommitteeFlow<CG, BC>>,
}

impl<CG, BC, FactoryBSF> EventProcessor for SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        info!("Processing user request: {:?}", req);
        match req {
            UserRequests::ApplyToStream(input) => {
                let flow_id = Uuid::new_v4();
                let mut flow = self.flow_factory.create_flow(flow_id);
                flow.next_step(StepData::ApplyToStreamInput(input.clone()))?;
                self.flows.insert(flow_id, flow);
            }
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                // TODO(ask-Fairgate) how do we distinguish between flows? I think BitVMX should add uuid into this
                let first_flow = self.flows.values_mut().next();
                match first_flow {
                    Some(flow) => flow.next_step(StepData::P2PAddress(comm_info.clone()))?,
                    None => bail!("No flow found for OutgoingBitVMXApiMessages::CommInfo"),
                }
            }
            OutgoingBitVMXApiMessages::PubKey(uuid, key) => {
                if let Some(flow) = self.flows.get_mut(uuid) {
                    flow.next_step(StepData::PublicKey(key.clone()))?;
                } else {
                    bail!("No flow found for OutgoingBitVMXApiMessages::PubKey and id {uuid}");
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        // TODO handle shutdown logic if necessary
    }
}

pub(crate) struct SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
}

impl<CG, BC> SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub(crate) fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
        }
    }
}

// TODO commonize with other flows
impl<CG, BC> SetupCommitteeFlowFactoryApi<CG, BC> for SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn create_flow(&self, flow_id: Uuid) -> SetupCommitteeFlow<CG, BC> {
        SetupCommitteeFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            flow_id,
        )
    }
}
