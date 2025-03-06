use crate::rsk_utils::generate_fake_tx_hash;
use common::types::{LogEvent, LogInfo, RskBlock, RskLog};
use sha3::{Digest, Keccak256};

/// A stateless generator for fake RSK logs.
#[derive(Clone)]
pub struct FakeLogGenerator {
    event_signature: String,
}

impl FakeLogGenerator {
    pub fn new(event_signature: &str) -> Self {
        Self {
            event_signature: event_signature.to_string(),
        }
    }

    pub fn generate_log(
        &self,
        block: RskBlock,
        tx_id: u64,
        address: String,
        log_index: u64,
    ) -> RskLog {
        let tx_hash = generate_fake_tx_hash(tx_id, address.as_str());
        let info: LogInfo = LogInfo::new(
            address,
            block.hash(),
            block.number(),
            tx_hash,
            log_index,
            false,
        );
        let topics = vec![];
        let event: LogEvent =
            LogEvent::new(event_signature_to_topic(&self.event_signature), topics);
        RskLog::new(info, event)
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
/// use test_utils::rsk_log_generator::event_signature_to_topic;
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
/// This function takes a hexadecimal address string (with or without the "0x" prefix),
/// verifies that it consists of 40 hexadecimal digits after stripping the prefix, and
/// then returns a topic string by prepending 24 zeros (to make up 64 hex digits in total
/// after the "0x").
///
/// # Panics
///
/// This function will panic if the provided address does not have exactly 40 hexadecimal
/// digits after removing the "0x" prefix.
///
/// # Parameters
///
/// - `address`: A string slice representing the Rootstock address.
///
/// # Returns
///
/// A `String` containing the topic derived from the address.
///
/// # Example
///
/// ```
/// use test_utils::rsk_log_generator::address_to_topic;
///
/// let topic = address_to_topic("0x1234567890abcdef1234567890abcdef12345678");
/// assert!(topic.starts_with("0x"));
/// ```
pub fn address_to_topic(address: &str) -> String {
    let addr = address.strip_prefix("0x").unwrap_or(address);
    if addr.len() != 40 {
        panic!(
            "Invalid Rootstock address length: expected 40 hex digits, got {}",
            addr.len()
        );
    }
    format!("0x{}{}", "0".repeat(24), addr)
}
