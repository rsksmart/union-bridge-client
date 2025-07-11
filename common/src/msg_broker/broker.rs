use crate::msg_broker::types::{FromServer, ToServer};
use bitvmx_client::types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use log::debug;
use message_broker::broker_memstorage::MemStorage;
use message_broker::channel::channel::{DualChannel, LocalChannel};
use message_broker::rpc::BrokerConfig;
use message_broker::rpc::sync_server::BrokerSync;
use mockall::automock;
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use thiserror::Error;

// by convention, server is id 1
pub const BROKER_SERVER_ID: u32 = 1;
pub const BITVMX_L2_BROKER_CLIENT_ID: u32 = 100; // Should match the ID defined in the BitVMX Client

#[automock]
pub trait BrokerServerApi<S: Serialize, C: Serialize> {
    fn send(&self, msg: &C, dst: u32) -> Result<(), BrokerError>;
    fn try_recv(&self) -> Result<Option<(S, u32)>, BrokerError>;
    fn close(&mut self);
}

#[automock]
pub trait BrokerClientApi<S: Serialize, C: Serialize> {
    fn send(&self, dest: u32, msg: S) -> Result<bool, BrokerError>;
    fn try_recv(&self) -> Result<Option<C>, BrokerError>;
}

/// Union-specific broker server implementation
pub struct BrokerServer {
    broker: BrokerSync,
    channel: LocalChannel<MemStorage>,
}

/// "Alias" for `BrokerServerApi<ToServer, FromServer>`
pub trait UnionBrokerServerApi: BrokerServerApi<ToServer, FromServer> {}
impl<T> UnionBrokerServerApi for T where T: BrokerServerApi<ToServer, FromServer> {}

/// "Alias" for `BrokerClientApi<ToServer, FromServer>`
pub trait UnionBrokerClientApi: BrokerClientApi<ToServer, FromServer> {}
impl<T> UnionBrokerClientApi for T where T: BrokerClientApi<ToServer, FromServer> {}

impl BrokerServer {
    pub fn new(port: u16) -> Self {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - change to disk storage (broker feature)
        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let broker_config = BrokerConfig::new(port, Some(IpAddr::from(Ipv4Addr::new(0, 0, 0, 0))));
        let broker = BrokerSync::new(&broker_config, broker_storage.clone());
        let broker_channel = LocalChannel::new(BROKER_SERVER_ID, broker_storage.clone());

        Self {
            broker,
            channel: broker_channel,
        }
    }
}

impl BrokerServerApi<ToServer, FromServer> for BrokerServer {
    fn try_recv(&self) -> Result<Option<(ToServer, u32)>, BrokerError> {
        if let Some((msg, sender)) = self
            .channel
            .recv()
            .map_err(BrokerError::BrokerServerError)?
        {
            let req = serde_json::from_str(&msg).map_err(BrokerError::SerializationError)?;
            Ok(Some((req, sender)))
        } else {
            Ok(None)
        }
    }

    fn send(&self, msg: &FromServer, dst: u32) -> Result<(), BrokerError> {
        self.channel
            .send(dst, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)?;
        Ok(())
    }

    fn close(&mut self) {
        self.broker.close();
    }
}

/// Union-specific broker client implementation
/// Do not make cloneable, use Arc instead. Reasons:
/// 1. cloning DualChannel can be considered expensive
/// 2. automock is not creating a cloneable MockBrokerClientApi
pub struct BrokerClient {
    channel: DualChannel,
}

impl BrokerClient {
    pub fn new(ip: IpAddr, port: u16, my_id: u32) -> Self {
        debug!("Starting BrokerClient on {ip}:{port} with id {my_id}");
        let broker_config = BrokerConfig::new(port, Some(ip));
        let client = DualChannel::new(&broker_config, my_id);
        Self { channel: client }
    }
}

impl BrokerClientApi<ToServer, FromServer> for BrokerClient {
    fn send(&self, dest: u32, msg: ToServer) -> Result<bool, BrokerError> {
        self.channel
            .send(dest, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)
    }

    fn try_recv(&self) -> Result<Option<FromServer>, BrokerError> {
        self.channel.recv()?.map_or(Ok(None), |(data, _id)| {
            serde_json::from_str(&data)
                .map(|deserialized| Some(deserialized))
                .map_err(BrokerError::SerializationError)
        })
    }
}

/// "Alias" for `BrokerServerApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>`
pub trait BitVmxBrokerServerApi:
    BrokerServerApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>
{
}
impl<T> BitVmxBrokerServerApi for T where
    T: BrokerServerApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>
{
}

/// "Alias" for `BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>`
pub trait BitVmxBrokerClientApi:
    BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>
{
}
impl<T> BitVmxBrokerClientApi for T where
    T: BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>
{
}

/// BitVMX-specific broker server implementation
pub struct BitVmxBrokerServer {
    broker: BrokerSync,
    channel: LocalChannel<MemStorage>,
}

impl BitVmxBrokerServer {
    pub fn new(port: u16) -> Self {
        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let broker_config = BrokerConfig::new(port, Some(IpAddr::from(Ipv4Addr::new(0, 0, 0, 0))));
        let broker = BrokerSync::new(&broker_config, broker_storage.clone());
        let broker_channel = LocalChannel::new(BROKER_SERVER_ID, broker_storage.clone());

        Self {
            broker,
            channel: broker_channel,
        }
    }
}

impl BrokerServerApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> for BitVmxBrokerServer {
    fn try_recv(&self) -> Result<Option<(IncomingBitVMXApiMessages, u32)>, BrokerError> {
        if let Some((msg, sender)) = self
            .channel
            .recv()
            .map_err(BrokerError::BrokerServerError)?
        {
            // For BitVMX server, we expect IncomingBitVMXApiMessages directly
            let req = serde_json::from_str::<IncomingBitVMXApiMessages>(&msg)
                .map_err(BrokerError::SerializationError)?;

            Ok(Some((req, sender)))
        } else {
            Ok(None)
        }
    }

    fn send(&self, msg: &OutgoingBitVMXApiMessages, dst: u32) -> Result<(), BrokerError> {
        self.channel
            .send(dst, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)?;
        Ok(())
    }

    fn close(&mut self) {
        self.broker.close();
    }
}

/// BitVMX-specific broker client implementation
/// Do not make cloneable, use Arc instead. Reasons:
/// 1. cloning DualChannel can be considered expensive
/// 2. automock is not creating a cloneable MockBrokerClientApi
pub struct BitVmxBrokerClient {
    channel: DualChannel,
}

impl BitVmxBrokerClient {
    pub fn new(ip: IpAddr, port: u16, my_id: u32) -> Self {
        debug!("Starting BitVmxBrokerClient on {ip}:{port} with id {my_id}");
        let broker_config = BrokerConfig::new(port, Some(ip));
        let client = DualChannel::new(&broker_config, my_id);
        Self { channel: client }
    }
}

impl BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> for BitVmxBrokerClient {
    fn send(&self, dest: u32, msg: IncomingBitVMXApiMessages) -> Result<bool, BrokerError> {
        self.channel
            .send(dest, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)
    }

    fn try_recv(&self) -> Result<Option<OutgoingBitVMXApiMessages>, BrokerError> {
        self.channel.recv()?.map_or(Ok(None), |(data, _id)| {
            let msg = serde_json::from_str::<OutgoingBitVMXApiMessages>(&data)
                .map_err(BrokerError::SerializationError)?;

            Ok(Some(msg))
        })
    }
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("Broker error: {0}")]
    BrokerServerError(#[from] message_broker::rpc::errors::BrokerError),
    #[error("Serialization error on Broker: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Unknown error on Broker: {0}")]
    UnknownError(#[from] anyhow::Error),
}
