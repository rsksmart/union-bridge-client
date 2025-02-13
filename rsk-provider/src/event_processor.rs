use serde_json::{json, Value};

pub(super) mod event_processor_abi;
pub(super) mod event_processor_typed;

pub(crate) fn build_event_json(name: &str, address: &str, decoded_input: Value) -> Value {
    let mut decoded_log = serde_json::Map::new();
    decoded_log.insert("name".to_string(), json!(name));
    decoded_log.insert("address".to_string(), json!(address));
    decoded_log.insert("input".to_string(), decoded_input.into());
    decoded_log.into()
}
