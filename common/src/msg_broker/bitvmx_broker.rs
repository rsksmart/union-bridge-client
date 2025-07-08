use crate::msg_broker::broker::{BROKER_SERVER_ID, BrokerClientApi, BrokerError, BrokerServerApi};
use bitvmx_client::types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use log::debug;
use message_broker::broker_memstorage::MemStorage;
use message_broker::channel::channel::{DualChannel, LocalChannel};
use message_broker::rpc::BrokerConfig;
use message_broker::rpc::sync_server::BrokerSync;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

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
#[derive(Clone)]
pub struct BitVmxBrokerClient {
    channel: Arc<DualChannel>,
}

impl BitVmxBrokerClient {
    pub fn new(ip: IpAddr, port: u16, my_id: u32) -> Self {
        debug!("Starting BitVmxBrokerClient on {ip}:{port} with id {my_id}");
        let broker_config = BrokerConfig::new(port, Some(ip));
        let client = DualChannel::new(&broker_config, my_id);
        Self {
            channel: Arc::new(client),
        }
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
