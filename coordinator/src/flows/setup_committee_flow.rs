use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::UserRequests;
use anyhow::{Result, bail};
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
#[cfg(test)]
use mockall::automock;
use transaction_dispatcher::types::ApplyToStreamInput;

#[derive(Debug, Clone)]
enum Steps {
    Step1(ApplyToStreamInput), // user input
    Step2(P2PAddress),         // comm info
    Step3(PublicKey),          // take key
    Step4(PublicKey),          // dispute key
}

impl Steps {
    fn num(&self) -> u8 {
        match self {
            Steps::Step1(_) => 1,
            Steps::Step2(_) => 2,
            Steps::Step3(_) => 3,
            Steps::Step4(_) => 4,
        }
    }

    fn run<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi>(
        &self,
        flow: &mut SetupCommitteeFlow<CG, BC>,
    ) -> Result<()> {
        match self {
            Steps::Step1(input) => flow.request_bitvmx_comm_info(input),
            Steps::Step2(_) => flow.request_bitvmx_take_pub_key(),
            Steps::Step3(_) => flow.request_bitvmx_dispute_pub_key(),
            Steps::Step4(_) => {
                todo!("not yet implemented")
            }
        }
    }
}

pub(crate) struct State {
    flow_id: Uuid,
    steps: Vec<Steps>,
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
                steps: vec![],
            },
        }
    }

    fn current_step(&mut self) -> u8 {
        self.state.steps.len() as u8
    }

    fn next_step(&mut self, step: Steps) -> Result<()> {
        if step.num() != self.current_step() + 1 {
            bail!("Invalid step change {step:?}");
        }

        step.run(self)?;

        self.state.steps.push(step);

        Ok(())
    }

    fn request_bitvmx_comm_info(&self, input: &ApplyToStreamInput) -> Result<()> {
        // TODO how do they know for whom to get comm info?
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo());
        Ok(())
    }

    fn request_bitvmx_take_pub_key(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetPubKey(
            self.state.flow_id,
            true,
        ));
        Ok(())
    }

    fn request_bitvmx_dispute_pub_key(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetPubKey(
            self.state.flow_id,
            true,
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
                flow.next_step(Steps::Step1(input.clone()))?;
                self.flows.insert(flow_id, flow);
            }
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                // TODO how do we distinguish between flows? I think BitVMX should add uuid into this
                let first_flow = self.flows.values_mut().next();
                match first_flow {
                    Some(flow) => flow.next_step(Steps::Step2(comm_info.clone()))?,
                    None => bail!("No flow found for OutgoingBitVMXApiMessages::CommInfo"),
                }
            }
            OutgoingBitVMXApiMessages::PubKey(uuid, key) => {
                if let Some(flow) = self.flows.get_mut(uuid) {
                    // required to distinguish between take key and dispute keys (same message under the hood)
                    if flow.current_step() == 2 {
                        flow.next_step(Steps::Step3(key.clone()))?;
                    } else if flow.current_step() == 3 {
                        flow.next_step(Steps::Step4(key.clone()))?;
                    } else {
                        bail!(
                            "Invalid step {} for OutgoingBitVMXApiMessages::PubKey",
                            flow.current_step()
                        );
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        // Handle shutdown logic if necessary
    }
}

// TODO commonize with other flows
#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowFactoryApi<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi>
{
    fn create_flow(&self, flow_id: Uuid) -> SetupCommitteeFlow<CG, BC>;
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
