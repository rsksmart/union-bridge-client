use crate::rsk_utilities::{
    address_to_topic, event_signature_to_topic, get_fake_address, get_fake_tx_hash,
};
use common::types::{LogEvent, LogInfo, RskBlock, RskLog};

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
            block.hash().to_string(),
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
