use crate::rsk_utils::{address_to_topic, get_fake_address, get_fake_tx_hash};
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
        address_num: u64,
        log_index: u64,
    ) -> RskLog {
        let address_from = get_fake_address(address_num, None);
        let address_to = get_fake_address(address_num, Some("destinatary"));
        let tx_hash = get_fake_tx_hash(tx_id, &address_from);
        let info: LogInfo = LogInfo::new(
            address_from.clone(),
            block.hash(),
            block.number(),
            tx_hash,
            log_index,
            false,
        );
        let topics = vec![
            address_to_topic(&address_from),
            address_to_topic(&address_to),
        ];
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
