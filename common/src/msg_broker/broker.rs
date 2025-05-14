use crate::msg_broker::types::{BrokerRequests, BrokerResponses};
use message_broker::broker_memstorage::MemStorage;
use message_broker::channel::channel::{DualChannel, LocalChannel};
use message_broker::rpc::BrokerConfig;
use message_broker::rpc::sync_server::BrokerSync;
use mockall::automock;
use std::sync::{Arc, Mutex};
use thiserror::Error;

// by convention, server is id 1
pub const BROKER_SERVER_ID: u32 = 1;

#[automock]
pub trait BrokerServerApi {
    fn try_recv(&self) -> Result<Option<(BrokerRequests, u32)>, BrokerError>;
    fn send(&self, msg: &BrokerResponses, dst: u32) -> Result<(), BrokerError>;
    fn close(&mut self);
}

#[automock]
pub trait BrokerClientApi {
    fn send(&self, dest: u32, msg: BrokerRequests) -> Result<bool, BrokerError>;
    fn try_recv(&self) -> Result<Option<BrokerResponses>, BrokerError>;
}

pub struct BrokerServer {
    broker: BrokerSync,
    channel: LocalChannel<MemStorage>,
}

impl BrokerServer {
    pub fn new(port: u16) -> Self {
        // TODO(Jira-CoordinatorResilience) change to disk storage
        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let broker_config = BrokerConfig::new(port, None);
        let broker = BrokerSync::new(&broker_config, broker_storage.clone());
        let broker_channel = LocalChannel::new(BROKER_SERVER_ID, broker_storage.clone());

        Self {
            broker,
            channel: broker_channel,
        }
    }
}

impl BrokerServerApi for BrokerServer {
    fn try_recv(&self) -> Result<Option<(BrokerRequests, u32)>, BrokerError> {
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

    fn send(&self, msg: &BrokerResponses, dst: u32) -> Result<(), BrokerError> {
        self.channel
            .send(dst, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)?;
        Ok(())
    }

    fn close(&mut self) {
        self.broker.close();
    }
}

pub struct BrokerClient {
    channel: DualChannel,
}

impl BrokerClient {
    pub fn new(port: u16, my_id: u32) -> Self {
        let broker_config = BrokerConfig::new(port, None);
        let client = DualChannel::new(&broker_config, my_id);
        Self { channel: client }
    }
}

impl BrokerClientApi for BrokerClient {
    fn send(&self, dest: u32, msg: BrokerRequests) -> Result<bool, BrokerError> {
        self.channel
            .send(dest, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)
    }

    fn try_recv(&self) -> Result<Option<BrokerResponses>, BrokerError> {
        self.channel.recv()?.map_or(Ok(None), |(data, _id)| {
            serde_json::from_str(&data)
                .map(|deserialized| Some(deserialized))
                .map_err(BrokerError::SerializationError)
        })
    }
}

#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("Broker error: {0}")]
    BrokerServerError(#[from] message_broker::rpc::errors::BrokerError),
    #[error("Serialization error on Broker: {0}")]
    SerializationError(#[from] serde_json::Error),
}
