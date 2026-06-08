use std::fs;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use broker_storage_backend::storage::Storage;
use broker_storage_backend::storage_config::StorageConfig;
use message_broker::broker_memstorage::MemStorage;
use message_broker::broker_storage::BrokerStorage;
use message_broker::channel::channel::{DualChannel, LocalChannel};
// Re-export for convenience - these are used in the public API
pub use message_broker::identification::allow_list::AllowList;
pub use message_broker::identification::identifier::Identifier;
pub use message_broker::identification::routing::RoutingTable;
use message_broker::identification::routing::WildCard;
use message_broker::rpc::BrokerConfig;
use message_broker::rpc::errors::BrokerError as RpcBrokerError;
use message_broker::rpc::sync_server::BrokerSync;
pub use message_broker::rpc::tls_helper::Cert;
use mockall::automock;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tracing::{debug, trace};

use crate::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use crate::msg_broker::types::{FromServer, ToServer};

// by convention, server is id 0 (matching bitvmx broker convention)
pub const BROKER_SERVER_ID: u8 = 0;
pub const BITVMX_L2_BROKER_CLIENT_ID: u8 = 0; // Should match the ID defined in the BitVMX Client
type SharedAllowList = Arc<Mutex<AllowList>>;
type SharedRoutingTable = Arc<Mutex<RoutingTable>>;

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
    channel: BrokerChannel,
}

enum BrokerChannel {
    InMemory(LocalChannel<MemStorage>),
    Persistent(LocalChannel<BrokerStorage>),
}

impl BrokerChannel {
    fn recv(
        &self,
    ) -> Result<Option<(String, Identifier)>, message_broker::rpc::errors::BrokerError> {
        match self {
            Self::InMemory(channel) => channel.recv(),
            Self::Persistent(channel) => channel.recv(),
        }
    }

    fn send(
        &self,
        dst: &Identifier,
        msg: String,
    ) -> Result<bool, message_broker::rpc::errors::BrokerError> {
        match self {
            Self::InMemory(channel) => channel.send(dst, msg),
            Self::Persistent(channel) => channel.send(dst, msg),
        }
    }
}

/// "Alias" for `BrokerServerApi<ToServer, FromServer>`
pub trait UnionBrokerServerApi: BrokerServerApi<ToServer, FromServer> {}
impl<T> UnionBrokerServerApi for T where T: BrokerServerApi<ToServer, FromServer> {}

/// "Alias" for `BrokerClientApi<ToServer, FromServer>`
pub trait UnionBrokerClientApi: BrokerClientApi<ToServer, FromServer> {}
impl<T> UnionBrokerClientApi for T where T: BrokerClientApi<ToServer, FromServer> {}

impl<S: Serialize, C: DeserializeOwned, T: BrokerClientApi<S, C>> BrokerClientApi<S, C>
    for std::rc::Rc<T>
{
    fn send(&self, msg: S) -> Result<bool, BrokerError> {
        self.as_ref().send(msg)
    }

    fn try_recv(&self) -> Result<Option<C>, BrokerError> {
        self.as_ref().try_recv()
    }
}

impl BrokerServer {
    /// Create a new `BrokerServer` that accepts broker messages from one authorized peer.
    /// Uses a deterministic identity from the provided key file.
    ///
    /// # Arguments
    /// * `port` - Port to listen on
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    /// * `authorized_peer` - Peer allowed to send messages to this broker server
    ///
    /// # Errors
    ///
    /// Returns an error if certificate loading fails or broker initialization fails.
    pub fn new(
        port: u16,
        key_path: &str,
        authorized_peer: &Identifier,
    ) -> Result<Self, BrokerError> {
        debug!("Starting BrokerServer on port {port}");

        let (cert, pubk_hash, broker_config) = broker_server_config(port, key_path)?;
        debug!("BrokerServer identity: pubkey_hash={pubk_hash}");

        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let server_identifier = Identifier::new(pubk_hash, BROKER_SERVER_ID);
        let (allow_list, routing) =
            broker_server_access_control(authorized_peer, &server_identifier)?;
        let broker =
            BrokerSync::new(&broker_config, broker_storage.clone(), cert, allow_list, routing)?;
        let broker_channel = LocalChannel::new(server_identifier, broker_storage.clone());

        Ok(Self { broker, channel: BrokerChannel::InMemory(broker_channel) })
    }

    /// Create a new `BrokerServer` with disk-backed broker queue storage.
    /// Uses a deterministic identity from the provided key file.
    ///
    /// # Arguments
    /// * `port` - Port to listen on
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    /// * `storage_path` - Path to the broker queue storage directory
    /// * `authorized_peer` - Peer allowed to send messages to this broker server
    ///
    /// # Errors
    ///
    /// Returns an error if certificate loading, storage initialization, or broker initialization
    /// fails.
    pub fn new_with_storage_path(
        port: u16,
        key_path: &str,
        storage_path: impl AsRef<Path>,
        authorized_peer: &Identifier,
    ) -> Result<Self, BrokerError> {
        debug!("Starting persistent BrokerServer on port {port}");

        let (cert, pubk_hash, broker_config) = broker_server_config(port, key_path)?;
        debug!("Persistent BrokerServer identity: pubkey_hash={pubk_hash}");

        let broker_storage = persistent_broker_storage(storage_path)?;
        let server_identifier = Identifier::new(pubk_hash, BROKER_SERVER_ID);
        let (allow_list, routing) =
            broker_server_access_control(authorized_peer, &server_identifier)?;
        let broker =
            BrokerSync::new(&broker_config, broker_storage.clone(), cert, allow_list, routing)?;
        let broker_channel = LocalChannel::new(server_identifier, broker_storage.clone());

        Ok(Self { broker, channel: BrokerChannel::Persistent(broker_channel) })
    }
}

fn broker_server_access_control(
    authorized_peer: &Identifier,
    server_identifier: &Identifier,
) -> Result<(SharedAllowList, SharedRoutingTable), BrokerError> {
    let allow_list = AllowList::new();
    allow_list
        .lock()
        .map_err(|error| broker_mutex_error("allow_list", &error))?
        .add_wildcard(authorized_peer.pubkey_hash.clone());

    let routing = RoutingTable::new();
    routing.lock().map_err(|error| broker_mutex_error("routing", &error))?.add_route(
        authorized_peer.clone(),
        server_identifier.clone(),
        WildCard::No,
    );

    Ok((allow_list, routing))
}

fn broker_mutex_error<T>(name: &str, error: &std::sync::PoisonError<T>) -> BrokerError {
    BrokerError::BrokerServerError(RpcBrokerError::MutexError(format!("{name}: {error}")))
}

fn broker_server_config(
    port: u16,
    key_path: &str,
) -> Result<(Cert, String, BrokerConfig), BrokerError> {
    let cert = Cert::from_key_file(key_path)?;
    let pubk_hash = cert.get_pubk_hash()?;
    let broker_config =
        BrokerConfig::new(port, Some(IpAddr::from(Ipv4Addr::UNSPECIFIED)), pubk_hash.clone());

    Ok((cert, pubk_hash, broker_config))
}

/// Derive a broker queue storage path from the indexer storage root and service name.
#[must_use]
pub fn broker_queue_storage_path(
    indexer_storage_path: impl AsRef<Path>,
    service_name: &str,
) -> PathBuf {
    indexer_storage_path.as_ref().join("broker").join(service_name)
}

fn persistent_broker_storage(
    storage_path: impl AsRef<Path>,
) -> Result<Arc<Mutex<BrokerStorage>>, BrokerError> {
    let storage_path = storage_path.as_ref();
    fs::create_dir_all(storage_path).with_context(|| {
        format!("Failed to create broker queue storage directory at {}", storage_path.display())
    })?;

    let storage_path_str = storage_path.to_str().ok_or_else(|| {
        anyhow::anyhow!("Broker queue storage path is not valid UTF-8: {}", storage_path.display())
    })?;
    let storage_config = StorageConfig::new(storage_path_str.to_owned(), None);
    let broker_backend = Storage::new(&storage_config).map_err(|error| {
        BrokerError::UnknownError(anyhow::anyhow!(
            "Failed to initialize broker queue storage at {}: {error}",
            storage_path.display()
        ))
    })?;
    let broker_backend = Arc::new(Mutex::new(broker_backend));

    Ok(Arc::new(Mutex::new(BrokerStorage::new(broker_backend))))
}

impl BrokerServerApi<ToServer, FromServer> for BrokerServer {
    fn try_recv(&self) -> Result<Option<(ToServer, Identifier)>, BrokerError> {
        if let Some((msg, sender)) = self.channel.recv().map_err(BrokerError::BrokerServerError)? {
            trace!("Received message from BrokerServer: {msg:?} from {sender:?}");
            let req = serde_json::from_str(&msg).map_err(BrokerError::SerializationError)?;
            Ok(Some((req, sender)))
        } else {
            Ok(None)
        }
    }

    fn send(&self, msg: &FromServer, dst: &Identifier) -> Result<(), BrokerError> {
        trace!("Sending message to BrokerServer: {msg:?} to {dst:?}");
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
/// 1. cloning `DualChannel` can be considered expensive
/// 2. automock is not creating a cloneable `MockBrokerClientApi`
pub struct BrokerClient {
    channel: DualChannel,
}

impl BrokerClient {
    /// Create a new `BrokerClient` in simple/testing mode with a deterministic `pubkey_hash`.
    /// The private key file is used to derive a stable `pubkey_hash` that other components
    /// can be configured with.
    ///
    /// # Arguments
    /// * `host` - Host to connect to
    /// * `port` - Port to connect to
    /// * `server_pubk_hash` - Public key hash of the server (can be empty for `allow_all` servers)
    /// * `my_id` - Client ID (u8)
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    ///
    /// # Panics
    ///
    /// Panics if the host cannot be resolved to an IP address.
    ///
    /// # Errors
    ///
    /// Returns an error if certificate loading fails or broker connection fails.
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
        let my_identifier = Identifier { pubkey_hash: my_cert.get_pubk_hash()?, id: my_id };

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
        trace!("Sending message to BrokerServer: {msg:?}");
        self.channel
            .send_server(serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)
    }

    fn try_recv(&self) -> Result<Option<FromServer>, BrokerError> {
        self.channel.recv()?.map_or(Ok(None), |(data, _id)| {
            serde_json::from_str(&data).map(Some).map_err(BrokerError::SerializationError)
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
    /// Create a new `BitVmxBrokerServer` with TLS support
    ///
    /// # Errors
    ///
    /// Returns a `BrokerError` if:
    /// - The public key hash cannot be extracted from the certificate
    /// - The underlying `BrokerSync` fails to initialize
    pub fn new(
        port: u16,
        cert: Cert,
        allow_list: Arc<Mutex<AllowList>>,
        routing: Arc<Mutex<RoutingTable>>,
    ) -> Result<Self, BrokerError> {
        debug!("Starting BitVmxBrokerServer on port {port}");

        let pubk_hash = cert.get_pubk_hash()?;
        let broker_storage = Arc::new(Mutex::new(MemStorage::new()));
        let broker_config =
            BrokerConfig::new(port, Some(IpAddr::from(Ipv4Addr::UNSPECIFIED)), pubk_hash.clone());
        let broker =
            BrokerSync::new(&broker_config, broker_storage.clone(), cert, allow_list, routing)?;

        let server_identifier = Identifier::new(pubk_hash, BROKER_SERVER_ID);
        let broker_channel = LocalChannel::new(server_identifier, broker_storage.clone());

        Ok(Self { broker, channel: broker_channel })
    }
}

impl BrokerServerApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> for BitVmxBrokerServer {
    fn try_recv(&self) -> Result<Option<(IncomingBitVMXApiMessages, Identifier)>, BrokerError> {
        if let Some((msg, sender)) = self.channel.recv().map_err(BrokerError::BrokerServerError)? {
            let req = serde_json::from_str::<IncomingBitVMXApiMessages>(&msg)
                .map_err(BrokerError::SerializationError)?;

            Ok(Some((req, sender)))
        } else {
            Ok(None)
        }
    }

    fn send(&self, msg: &OutgoingBitVMXApiMessages, dst: &Identifier) -> Result<(), BrokerError> {
        trace!("Sending message to BitVMX: {msg:?} to {dst:?}");
        self.channel
            .send(dst, serde_json::to_string(&msg)?)
            .map_err(BrokerError::BrokerServerError)?;
        Ok(())
    }

    fn close(&mut self) {
        self.broker.close();
    }
}

/// `BitVMX`-specific broker client implementation
/// This client connects to the `BitVMX` broker server running in bitvmx-client
/// Do not make cloneable, use Arc instead. Reasons:
/// 1. cloning `DualChannel` can be considered expensive
/// 2. automock is not creating a cloneable `MockBrokerClientApi`
pub struct BitVmxBrokerClient {
    channel: DualChannel,
}

impl BitVmxBrokerClient {
    /// Create a new `BitVmxBrokerClient` with a deterministic identity from a key file.
    ///
    /// # Arguments
    /// * `host` - Host to connect to (bitvmx broker server)
    /// * `port` - Port to connect to
    /// * `server_pubk_hash` - Public key hash of the bitvmx broker server
    /// * `my_id` - Client ID (u8)
    /// * `key_path` - Path to PEM file containing the private key for deterministic identity
    /// # Panics
    /// Panics if the host cannot be resolved to an IP address.
    /// # Errors
    /// Returns an error if certificate loading fails or broker connection fails.
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
        trace!("Sending message to BitVMX: {msg:?}");
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

impl BrokerError {
    #[must_use]
    pub const fn disconnected() -> Self {
        Self::BrokerServerError(RpcBrokerError::Disconnected)
    }

    #[must_use]
    fn is_recoverable_transport_error(&self) -> bool {
        matches!(self, Self::BrokerServerError(error) if is_recoverable_rpc_broker_error(error))
    }
}

#[must_use]
pub fn is_recoverable_transport_error(error: &(dyn std::error::Error + 'static)) -> bool {
    error.downcast_ref::<BrokerError>().is_some_and(BrokerError::is_recoverable_transport_error)
        || error.downcast_ref::<RpcBrokerError>().is_some_and(is_recoverable_rpc_broker_error)
}

const fn is_recoverable_rpc_broker_error(error: &RpcBrokerError) -> bool {
    matches!(
        error,
        RpcBrokerError::Disconnected
            | RpcBrokerError::IoError(_)
            | RpcBrokerError::RpcError(_)
            | RpcBrokerError::ClosedChannel
    )
}

fn resolve_ip(name: String, port: u16) -> std::io::Result<IpAddr> {
    // ToSocketAddrs triggers DNS lookup via /etc/resolv.conf inside the container
    (name, port)
        .to_socket_addrs()?
        .find(std::net::SocketAddr::is_ipv4) // pick IPv4 if you need IpAddr::V4
        .map(|a| a.ip())
        .ok_or_else(|| std::io::Error::other("no A record"))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_broker_queue_storage_path() {
        assert_eq!(
            PathBuf::from("/tmp/indexer/broker/block-indexer"),
            broker_queue_storage_path("/tmp/indexer", "block-indexer"),
        );
    }

    #[test]
    fn test_persistent_broker_storage_creates_storage_directory() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let storage_path = temp_dir.path().join("broker").join("block-indexer");

        let _storage = persistent_broker_storage(&storage_path)
            .expect("Failed to create persistent broker storage");

        assert!(storage_path.is_dir());
    }

    #[test]
    fn test_broker_server_access_control_authorizes_only_expected_peer_route() {
        let authorized_peer = Identifier::new("peer".to_string(), 101);
        let server_identifier = Identifier::new("server".to_string(), BROKER_SERVER_ID);
        let other_peer = Identifier::new("other".to_string(), 101);

        let (allow_list, routing) =
            broker_server_access_control(&authorized_peer, &server_identifier)
                .expect("access control should build");

        let allow_list = allow_list.lock().expect("allow list lock should succeed");
        assert!(allow_list.is_allowed_by_fingerprint(&authorized_peer.pubkey_hash));
        assert!(!allow_list.is_allowed_by_fingerprint(&other_peer.pubkey_hash));
        drop(allow_list);

        let routing = routing.lock().expect("routing lock should succeed");
        assert!(routing.can_route(&authorized_peer, &server_identifier));
        assert!(!routing.can_route(&server_identifier, &authorized_peer));
        assert!(!routing.can_route(&other_peer, &server_identifier));
    }

    #[test]
    fn test_recoverable_broker_transport_error_accepts_runtime_disconnects() {
        let disconnected = BrokerError::BrokerServerError(RpcBrokerError::Disconnected);
        let closed_channel = BrokerError::BrokerServerError(RpcBrokerError::ClosedChannel);
        let io_error = BrokerError::BrokerServerError(RpcBrokerError::IoError(
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset"),
        ));

        assert!(is_recoverable_transport_error(&disconnected));
        assert!(is_recoverable_transport_error(&closed_channel));
        assert!(is_recoverable_transport_error(&io_error));
    }

    #[test]
    fn test_recoverable_broker_transport_error_rejects_non_runtime_failures() {
        let serialization_error =
            serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON should fail");
        let serialization = BrokerError::SerializationError(serialization_error);
        let tls = BrokerError::BrokerServerError(RpcBrokerError::TlsError("bad cert".to_string()));
        let unknown = BrokerError::UnknownError(anyhow::anyhow!("unexpected"));

        assert!(!is_recoverable_transport_error(&serialization));
        assert!(!is_recoverable_transport_error(&tls));
        assert!(!is_recoverable_transport_error(&unknown));
    }
}
