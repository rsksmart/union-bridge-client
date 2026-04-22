use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use anyhow::Result;
use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::msg_broker::types::{MemberFundingInfo, ToServer};
use log::{info, trace};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use crate::event_processor::EventProcessor;
use crate::types::UserRequests;

pub(crate) struct FundingInfoProcessor<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    bitvmx_broker: Rc<BC>,
    contracts: Rc<CG>,
    reply_tx: Sender<ToServer>,
    pending_requests: HashSet<Uuid>,
}

impl<BC, CG> FundingInfoProcessor<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    pub fn new(bitvmx_broker: Rc<BC>, contracts: Rc<CG>, reply_tx: Sender<ToServer>) -> Self {
        Self { bitvmx_broker, contracts, reply_tx, pending_requests: HashSet::new() }
    }
}

impl<BC, CG> EventProcessor for FundingInfoProcessor<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    fn process_user_request(&mut self, event: &UserRequests) -> Result<()> {
        match event {
            UserRequests::FundingInfo(req_id) => {
                self.pending_requests.insert(*req_id);
                self.bitvmx_broker.send(IncomingBitVMXApiMessages::GetFundingAddress(*req_id))?;
            }
            UserRequests::ApplyToStream(_) => {
                trace!("FundingInfoProcessor: Ignoring user request {event:?}");
            }
        }
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::FundingAddress(req_id, addr) => {
                if !self.pending_requests.remove(req_id) {
                    return Ok(());
                }

                let address = addr.clone().assume_checked().to_string();
                info!("Received BitVMX Funding Address: {address}");
                let _ = self.reply_tx.send(ToServer::MemberFundingInfo(
                    *req_id,
                    MemberFundingInfo {
                        bitcoin_address: address,
                        rsk_address: self.contracts.my_address().to_string(),
                    },
                ));
            }
            OutgoingBitVMXApiMessages::WalletError(req_id, message) => {
                if !self.pending_requests.remove(req_id) {
                    return Ok(());
                }

                let _ = self.reply_tx.send(ToServer::BitVmxWalletError(*req_id, message.clone()));
            }
            OutgoingBitVMXApiMessages::WalletNotReady(req_id) => {
                if !self.pending_requests.remove(req_id) {
                    return Ok(());
                }

                let _ = self.reply_tx.send(ToServer::BitVmxWalletError(
                    *req_id,
                    "BitVMX wallet is not ready".to_string(),
                ));
            }
            _ => {
                trace!("FundingInfoProcessor: Ignoring BitVMX event {event:?}");
            }
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        // nothing special to do here
    }
}
