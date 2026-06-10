use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::Result;
use bitcoin::Network;
use common_bitvmx::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common_broker::broker::BitVmxBrokerClientApi;
use common_broker::types::{MemberFundingInfo, ToServer};
use tracing::{info, trace, warn};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use crate::event_processor::EventProcessor;
use crate::types::UserRequests;

// Bound how long we keep a request pending before evicting it. The user-api
// caller times out after ~9s; a 60s ceiling here is enough slack to receive a
// late reply while guaranteeing the set cannot grow indefinitely if BitVMX
// never responds.
const PENDING_REQUEST_TTL: Duration = Duration::from_mins(1);

pub(crate) struct FundingInfoProcessor<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    bitvmx_broker: Rc<BC>,
    contracts: Rc<CG>,
    bitcoin_network: Network,
    reply_tx: Sender<ToServer>,
    pending_requests: HashMap<Uuid, Instant>,
}

impl<BC, CG> FundingInfoProcessor<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    pub(crate) fn new(
        bitvmx_broker: Rc<BC>,
        contracts: Rc<CG>,
        bitcoin_network: Network,
        reply_tx: Sender<ToServer>,
    ) -> Self {
        Self {
            bitvmx_broker,
            contracts,
            bitcoin_network,
            reply_tx,
            pending_requests: HashMap::new(),
        }
    }

    fn evict_expired(&mut self) {
        let now = Instant::now();
        self.pending_requests
            .retain(|_, started| now.duration_since(*started) < PENDING_REQUEST_TTL);
    }

    fn forward_reply(reply_tx: &Sender<ToServer>, reply: ToServer) {
        if let Err(err) = reply_tx.send(reply) {
            // Channel closure happens during coordinator shutdown; surface it
            // so a misrouted reply isn't lost without trace.
            warn!("Failed to forward reply to user-api channel: {err}");
        }
    }
}

impl<BC, CG> EventProcessor for FundingInfoProcessor<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    fn process_user_request(&mut self, event: &UserRequests) -> Result<()> {
        self.evict_expired();
        match event {
            UserRequests::FundingInfo(req_id) => {
                self.pending_requests.insert(*req_id, Instant::now());
                self.bitvmx_broker.send(IncomingBitVMXApiMessages::GetFundingAddress(*req_id))?;
            }
            UserRequests::ApplyToStream(_) | UserRequests::Admin(_) => {
                trace!("FundingInfoProcessor: Ignoring user request {event:?}");
            }
        }
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        self.evict_expired();
        match event {
            OutgoingBitVMXApiMessages::FundingAddress(req_id, addr) => {
                if self.pending_requests.remove(req_id).is_none() {
                    return Ok(());
                }

                let checked = match addr.clone().require_network(self.bitcoin_network) {
                    Ok(checked) => checked,
                    Err(err) => {
                        warn!(
                            "BitVMX returned funding address for the wrong network \
                             (expected {:?}): {err}",
                            self.bitcoin_network
                        );
                        Self::forward_reply(
                            &self.reply_tx,
                            ToServer::BitVmxWalletError(
                                *req_id,
                                format!(
                                    "BitVMX funding address does not match expected network {:?}",
                                    self.bitcoin_network
                                ),
                            ),
                        );
                        return Ok(());
                    }
                };

                let address = checked.to_string();
                info!("Received BitVMX Funding Address: {address}");
                Self::forward_reply(
                    &self.reply_tx,
                    ToServer::MemberFundingInfo(
                        *req_id,
                        MemberFundingInfo {
                            bitcoin_address: address,
                            rsk_address: self.contracts.my_address().to_string(),
                        },
                    ),
                );
            }
            OutgoingBitVMXApiMessages::WalletError(req_id, message) => {
                if self.pending_requests.remove(req_id).is_none() {
                    return Ok(());
                }

                Self::forward_reply(
                    &self.reply_tx,
                    ToServer::BitVmxWalletError(*req_id, message.clone()),
                );
            }
            OutgoingBitVMXApiMessages::WalletNotReady(req_id) => {
                if self.pending_requests.remove(req_id).is_none() {
                    return Ok(());
                }

                Self::forward_reply(
                    &self.reply_tx,
                    ToServer::BitVmxWalletError(*req_id, "BitVMX wallet is not ready".to_string()),
                );
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
