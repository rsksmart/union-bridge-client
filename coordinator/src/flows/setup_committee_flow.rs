use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::UserRequests;
use anyhow::Result;
use common::runtime_sync::RuntimeSync;
use log::info;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowApi {
    // TODO
}

pub(crate) struct State {
    pub(crate) flow_id: Uuid,
}

pub(crate) struct SetupCommitteeFlow<CG: RskContractsGatewayApi> {
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    blockchain_view: Rc<RefCell<BlockchainView>>,
    state: State,
}

impl<CG: RskContractsGatewayApi> SetupCommitteeFlow<CG> {
    fn new(contracts: Rc<CG>, rt_sync: RuntimeSync, flow_id: Uuid) -> Self {
        Self {
            contracts,
            rt_sync,
            blockchain_view: Rc::new(RefCell::new(BlockchainView::new())),
            state: State { flow_id },
        }
    }
}

pub(crate) struct SetupCommitteeProcessor<BSF, FactoryBSF>
where
    BSF: SetupCommitteeFlowApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<BSF>,
{
    flow_factory: FactoryBSF,
    flows: HashMap<Uuid, BSF>,
}

impl<CG: RskContractsGatewayApi> SetupCommitteeFlowApi for SetupCommitteeFlow<CG> {
    // TODO
}

impl<BSF, FactoryBSF> EventProcessor for SetupCommitteeProcessor<BSF, FactoryBSF>
where
    BSF: SetupCommitteeFlowApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<BSF>,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        info!("Processing user request: {:?}", req);
        Ok(())
    }

    fn shutdown(&mut self) {
        // Handle shutdown logic if necessary
    }
}

// TODO(iago) this can me moved to common for all flows
#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowFactoryApi<BSF: SetupCommitteeFlowApi> {
    fn create_flow(&self, flow_id: Uuid) -> BSF;
}

pub(crate) struct SetupCommitteeFlowFactory<CG: RskContractsGatewayApi> {
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
}

impl<CG: RskContractsGatewayApi> SetupCommitteeFlowFactory<CG> {
    pub(crate) fn new(contracts_gateway: Rc<CG>, rt_sync: RuntimeSync) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
        }
    }
}

impl<CG: RskContractsGatewayApi> SetupCommitteeFlowFactoryApi<SetupCommitteeFlow<CG>>
    for SetupCommitteeFlowFactory<CG>
{
    fn create_flow(&self, flow_id: Uuid) -> SetupCommitteeFlow<CG> {
        SetupCommitteeFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            flow_id,
        )
    }
}
