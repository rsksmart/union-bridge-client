use crate::event_processor::EventProcessor;
use crate::types::RskPegManagerEvents;
use common::msg_broker::{
    broker::BrokerClientApi,
    types::{FromServer, ToServer},
};
use common::types::RskBlockAndUncles;
use log::{debug, info, trace};
use reqwest::blocking::Client;

pub struct PegoutProcessor<BC: BrokerClientApi> {
    http_client: Client,
    bitvmx_broker: BC,
}

impl<BC: BrokerClientApi> PegoutProcessor<BC> {
    pub fn new(bitvmx_broker: BC) -> Self {
        Self {
            http_client: Client::new(),
            bitvmx_broker,
        }
    }
}

impl<T: BrokerClientApi> EventProcessor for PegoutProcessor<T> {
    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> anyhow::Result<()> {
        match event {
            FromServer::RegisterPegoutSignature(event) => {
                debug!("Pegout signature request received: {:?}", event);
                //Todo call signature contract
            }
            FromServer::RegisterPegout(event) => {
                debug!("Register Pegout request received: {:?}", event);
                //call PegManage contract
            }
            _ => (),
        }
        info!("Processing new bitvmx event: {:?}", event);
        Ok(())
    }

    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> anyhow::Result<()> {
        trace!("Processing new event: {:?}", event);
        match event {
            RskPegManagerEvents::PegoutRequested(event) => {
                debug!("Handling Pegout Requested event {:?}", event);
                //re-send to bitvmx
            }
            RskPegManagerEvents::PegoutRegistered(event) => {
                debug!("Handling Pegout Registered event {:?}", event);
            }
            _ => (),
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> anyhow::Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutProcessor");
    }
}
