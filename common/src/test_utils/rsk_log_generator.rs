use primitive_types::H256;
use sha3::{Digest, Keccak256};

use crate::types::{
    Address, BlockHash, DataBytes, Hash256, LogEvent, LogInfo, LogTopic, RskLog, TxHash,
};

/// A stateless generator for fake RSK logs.
#[derive(Clone)]
pub struct FakeLogGenerator {}

impl FakeLogGenerator {
    pub fn new() -> Self {
        FakeLogGenerator {}
    }
    pub fn generate_log_with_info(&self, event_signature: &str, log_info: LogInfo) -> RskLog {
        let address = log_info.address();
        let topics: Vec<Hash256> = vec![address_to_topic(address)];
        let event_signature_topic = DataBytes::new(event_signature.as_bytes().to_vec());
        let event: LogEvent = LogEvent::new(event_signature_topic, topics);
        RskLog::new(log_info, event)
    }

    pub fn generate_log(&self, event_signature: &str, address: Address) -> RskLog {
        let fake_log_info = LogInfo::new(
            address.clone(),
            BlockHash::from(H256::random()),
            1.into(),
            TxHash::from(H256::random()),
            1,
            false,
        );

        self.generate_log_with_info(event_signature, fake_log_info)
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
/// assert_eq!(topic.to_string(), "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
/// ```
pub fn event_signature_to_topic(event_signature: &str) -> LogTopic {
    let mut hasher = Keccak256::new();
    hasher.update(event_signature.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    LogTopic::from(H256::from(hash))
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
/// assert_eq!(topic.to_string(), "0x0000000000000000000000001234567890abcdef1234567890abcdef12345678");
/// ```
pub fn address_to_topic(address: Address) -> LogTopic {
    LogTopic::from(address.value())
}
