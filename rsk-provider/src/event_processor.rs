use anyhow::Result;
use common::types::RskLog;
use serde_json::{json, Value};

pub(super) mod event_processor_abi;
pub(super) mod event_processor_typed;

pub(crate) fn build_event_json(
    name: &str,
    rsk_log: &RskLog,
    decoded_input: Value,
) -> Result<Value> {
    let mut decoded_log = serde_json::Map::new();

    let log_info = serde_json::to_value(rsk_log.data())?;
    decoded_log.insert("log".to_string(), log_info);

    decoded_log.insert("name".to_string(), json!(name));
    decoded_log.insert("input".to_string(), decoded_input.into());

    Ok(decoded_log.into())
}
