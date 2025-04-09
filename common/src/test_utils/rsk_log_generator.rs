use crate::types::{Address, LogEvent, LogInfo, RskLog};
use sha3::{Digest, Keccak256};

/// A stateless generator for fake RSK logs.
#[derive(Clone)]
pub struct FakeLogGenerator {}

impl FakeLogGenerator {
    pub fn new() -> Self {
        FakeLogGenerator {}
    }
    pub fn generate_log(&self, event_signature: &str, log_info: LogInfo) -> RskLog {
        let address = log_info.address();
        let topics = vec![address_to_topic(address)];
        let event: LogEvent = LogEvent::new(event_signature_to_topic(event_signature), topics);
        RskLog::new(log_info, event)
    }
}

/// Converts an event signature to a topic hash using Keccak256.
///
/// This function takes an event signature (for example, `"Transfer(address,address,uint256)"`),
/// computes its Keccak256 hash, and returns the hash formatted as a hexadecimal string
/// prefixed with "0x".
///
/// # Parameters
///
/// - `event_signature`: A string slice representing the event signature.
///
/// # Returns
///
/// A `String` containing the topic hash derived from the event signature.
///
/// # Example
///
/// ```
/// use common::test_utils::rsk_log_generator::event_signature_to_topic;
///
/// let topic = event_signature_to_topic("Transfer(address,address,uint256)");
/// assert!(topic.starts_with("0x"));
/// ```
pub fn event_signature_to_topic(event_signature: &str) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(event_signature.as_bytes());
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(hash))
}

/// Converts a Rootstock address into a topic by left-padding it with zeros.
///
/// This function takes an `Address` type, retrieves its hexadecimal representation,
/// and returns a topic string by prepending 24 zeros (to make up 64 hex digits in total
/// after the "0x").
///
/// # Parameters
///
/// - `address`: An `Address` type representing the Rootstock address.
///
/// # Returns
///
/// A `String` containing the topic derived from the address.
///
/// # Example
///
/// ```
/// use common::types::Address;
/// use common::test_utils::rsk_log_generator::address_to_topic;
///
/// let address = Address::try_from("0x1234567890abcdef1234567890abcdef12345678").unwrap();
/// let topic = address_to_topic(address);
/// assert!(topic.starts_with("0x"));
/// ```
pub fn address_to_topic(address: Address) -> String {
    let addr_hex = hex::encode(address.value());
    format!("0x{}{}", "0".repeat(24), addr_hex)
}
