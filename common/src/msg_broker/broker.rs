use crate::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use crate::msg_broker::types::{FromServer, ToServer};
use log::debug;
use message_broker::broker_memstorage::MemStorage;
use message_broker::channel::channel::{DualChannel, LocalChannel};
use message_broker::rpc::BrokerConfig;
use message_broker::rpc::sync_server::BrokerSync;
use mockall::automock;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use thiserror::Error;

// Re-export for convenience - these are used in the public API
pub use message_broker::identification::allow_list::AllowList;
pub use message_broker::identification::identifier::Identifier;
pub use message_broker::identification::routing::RoutingTable;
pub use message_broker::rpc::tls_helper::Cert;

// by convention, server is id 0 (matching bitvmx broker convention)
pub const BROKER_SERVER_ID: u8 = 0;
pub const BITVMX_L2_BROKER_CLIENT_ID: u8 = 0; // Should match the ID defined in the BitVMX Client

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-213

#[automock]
pub trait BrokerServerApi<S: Serialize, C: DeserializeOwned> {
    /// # Errors
    ///
    /// Returns an error if the send operation fails.
    fn send(&self, msg: &C, dst: &Identifier) -> Result<(), BrokerError>;
    /// # Errors
    ///
    /// Returns an error if the receive operation fails.
    fn try_recv(&self) -> Result<Option<(S, Identifier)>, BrokerError>;
    fn close(&mut self);
}

#[automock]
pub trait BrokerClientApi<S: Serialize, C: DeserializeOwned> {
    /// # Errors
    ///
    /// Returns an error if the send operation fails.
    fn send(&self, msg: S) -> Result<bool, BrokerError>;
    /// # Errors
    ///
    /// Returns an error if the receive operation fails.
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
    /// Create a new BrokerServer in simple/testing mode (allow all connections)
    /// Uses a deterministic identity from the provided key file.
    ///
    /// # Arguments
    /// * `port` - Port to listen on
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    #[must_use]
    pub fn new(port: u16, key_path: &str) -> Result<Self, BrokerError> {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - change to disk storage (broker feature)
        debug!("Starting BrokerServer on port {port}");

        let cert = Cert::from_key_file(key_path)?;
        let pubk_hash = cert.get_pubk_hash()?;

        debug!("BrokerServer identity: pubkey_hash={}", pubk_hash);

        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let broker_config = BrokerConfig::new(
            port,
            Some(IpAddr::from(Ipv4Addr::new(0, 0, 0, 0))),
            pubk_hash.clone(),
        );
        let broker = BrokerSync::new_simple(&broker_config, broker_storage.clone(), cert)?;

        let server_identifier = Identifier::new(pubk_hash, BROKER_SERVER_ID);
        let broker_channel = LocalChannel::new(server_identifier, broker_storage.clone());

        Ok(Self {
            broker,
            channel: broker_channel,
        })
    }
}

impl BrokerServerApi<ToServer, FromServer> for BrokerServer {
    fn try_recv(&self) -> Result<Option<(ToServer, Identifier)>, BrokerError> {
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

    fn send(&self, msg: &FromServer, dst: &Identifier) -> Result<(), BrokerError> {
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
    /// Create a new BrokerClient in simple/testing mode with a deterministic pubkey_hash.
    /// The private key file is used to derive a stable pubkey_hash that other components
    /// can be configured with.
    ///
    /// # Arguments
    /// * `host` - Host to connect to
    /// * `port` - Port to connect to
    /// * `server_pubk_hash` - Public key hash of the server (can be empty for allow_all servers)
    /// * `my_id` - Client ID (u8)
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    pub fn new(
        host: String,
        port: u16,
        server_pubk_hash: String,
        my_id: u8,
        key_path: &str,
    ) -> Result<Self, BrokerError> {
        debug!("Starting BrokerClient on {host}:{port}");

        let ip = resolve_ip(host, port).expect("Unable to resolve IP");
        let broker_config = BrokerConfig::new(port, Some(ip), server_pubk_hash);

        let allow_list = AllowList::new();
        allow_list
            .lock()
            .map_err(|e| {
                BrokerError::BrokerServerError(
                    message_broker::rpc::errors::BrokerError::MutexError(e.to_string()),
                )
            })?
            .allow_all();

        let my_cert = Cert::from_key_file(key_path)?;
        let my_identifier = Identifier {
            pubkey_hash: my_cert.get_pubk_hash()?,
            id: my_id,
        };

        debug!(
            "BrokerClient identity: pubkey_hash={}, id={}",
            my_identifier.pubkey_hash, my_identifier.id
        );

        let client = DualChannel::new(&broker_config, my_cert, Some(my_id), allow_list)?;
        Ok(Self { channel: client })
    }
}

impl BrokerClientApi<ToServer, FromServer> for BrokerClient {
    fn send(&self, msg: ToServer) -> Result<bool, BrokerError> {
        self.channel
            .send_server(serde_json::to_string(&msg)?)
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
    /// Create a new BitVmxBrokerServer with TLS support
    #[must_use]
    pub fn new(
        port: u16,
        cert: Cert,
        allow_list: Arc<Mutex<AllowList>>,
        routing: Arc<Mutex<RoutingTable>>,
    ) -> Result<Self, BrokerError> {
        debug!("Starting BitVmxBrokerServer on port {port}");

        let pubk_hash = cert.get_pubk_hash()?;
        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let broker_config = BrokerConfig::new(
            port,
            Some(IpAddr::from(Ipv4Addr::new(0, 0, 0, 0))),
            pubk_hash.clone(),
        );
        let broker = BrokerSync::new(
            &broker_config,
            broker_storage.clone(),
            cert,
            allow_list,
            routing,
        )?;

        let server_identifier = Identifier::new(pubk_hash, BROKER_SERVER_ID);
        let broker_channel = LocalChannel::new(server_identifier, broker_storage.clone());

        Ok(Self {
            broker,
            channel: broker_channel,
        })
    }
}

impl BrokerServerApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> for BitVmxBrokerServer {
    fn try_recv(&self) -> Result<Option<(IncomingBitVMXApiMessages, Identifier)>, BrokerError> {
        if let Some((msg, sender)) = self
            .channel
            .recv()
            .map_err(BrokerError::BrokerServerError)?
        {
            let req = serde_json::from_str::<IncomingBitVMXApiMessages>(&msg)
                .map_err(BrokerError::SerializationError)?;

            Ok(Some((req, sender)))
        } else {
            Ok(None)
        }
    }

    fn send(&self, msg: &OutgoingBitVMXApiMessages, dst: &Identifier) -> Result<(), BrokerError> {
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
/// This client connects to the BitVMX broker server running in bitvmx-client
/// Do not make cloneable, use Arc instead. Reasons:
/// 1. cloning DualChannel can be considered expensive
/// 2. automock is not creating a cloneable MockBrokerClientApi
pub struct BitVmxBrokerClient {
    channel: DualChannel,
}

impl BitVmxBrokerClient {
    /// Create a new BitVmxBrokerClient with a deterministic identity from a key file.
    ///
    /// # Arguments
    /// * `host` - Host to connect to (bitvmx broker server)
    /// * `port` - Port to connect to
    /// * `server_pubk_hash` - Public key hash of the bitvmx broker server
    /// * `my_id` - Client ID (u8)
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    pub fn new(
        host: String,
        port: u16,
        server_pubk_hash: String,
        my_id: u8,
        key_path: &str,
    ) -> Result<Self, BrokerError> {
        debug!("Starting BitVmxBrokerClient on {host}:{port} with id {my_id}");

        let ip = resolve_ip(host, port).expect("Unable to resolve IP");
        let broker_config = BrokerConfig::new(port, Some(ip), server_pubk_hash);

        let allow_list = AllowList::new();
        allow_list
            .lock()
            .map_err(|e| {
                BrokerError::BrokerServerError(
                    message_broker::rpc::errors::BrokerError::MutexError(e.to_string()),
                )
            })?
            .allow_all();

        let my_cert = Cert::from_key_file(key_path)?;

        debug!(
            "BitVmxBrokerClient identity: pubkey_hash={}",
            my_cert.get_pubk_hash().unwrap_or_default()
        );

        let client = DualChannel::new(&broker_config, my_cert, Some(my_id), allow_list)?;
        Ok(Self { channel: client })
    }
}

impl BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> for BitVmxBrokerClient {
    fn send(&self, msg: IncomingBitVMXApiMessages) -> Result<bool, BrokerError> {
        self.channel
            .send_server(serde_json::to_string(&msg)?)
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

fn resolve_ip(name: String, port: u16) -> std::io::Result<IpAddr> {
    // ToSocketAddrs triggers DNS lookup via /etc/resolv.conf inside the container
    (name, port)
        .to_socket_addrs()?
        .find(|a| a.is_ipv4()) // pick IPv4 if you need IpAddr::V4
        .map(|a| a.ip())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "no A record"))
}
