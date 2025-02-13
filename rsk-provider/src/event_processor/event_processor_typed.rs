use crate::event_processor::build_event_json;
use alloy_primitives::{hex, LogData};
use alloy_sol_types::private::B256;
use alloy_sol_types::{sol, SolEvent};
use anyhow::{bail, Result};
use common::types::RskLog;
use log::error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;

// define here known events, they are converted to Rust structs (mapped types, etc.)
// for now these are just examples
sol! {
    #[derive(Serialize, Deserialize, Debug)]
    event ValueUpdate(
        uint256 value,
        bytes32 dataFeedId,
        uint256 updatedAt
    );
    #[derive(Serialize, Deserialize, Debug)]
    event LogValue(bytes32 val);
}

pub fn process(rsk_log: &RskLog) -> Result<Option<Value>> {
    let parsed_topics: Vec<B256> = rsk_log
        .event()
        .topics()
        .iter()
        .filter_map(|topic| topic.parse::<B256>().ok())
        .collect();

    let log_data = LogData::new(parsed_topics, hex::decode(&rsk_log.event().data())?.into());
    if log_data.is_none() {
        error!("Failed to parse log data: {:?}", log_data);
    }
    let log_data = log_data.unwrap();

    let topic0 = log_data.topics()[0];
    let event_name_and_input = match topic0 {
        ev if ev == ValueUpdate::SIGNATURE_HASH => {
            Some(decode_event_input::<ValueUpdate>(&log_data)?)
        }
        ev if ev == LogValue::SIGNATURE_HASH => Some(decode_event_input::<LogValue>(&log_data)?),
        // other types here in the future
        _ => None,
    };

    if event_name_and_input.is_none() {
        return Ok(None);
    }

    let (name, decoded_log_input) = event_name_and_input.unwrap();

    let event_json = build_event_json(&name, &rsk_log, decoded_log_input.into())?;
    Ok(Some(event_json))
}

fn decode_event_input<T: SolEvent + Serialize + Debug>(
    log_data: &LogData,
) -> Result<(&str, Value)> {
    let decoded_log = T::decode_log_data(&log_data, true);

    match decoded_log {
        Ok(input) => {
            let name = std::any::type_name::<T>()
                .rsplit("::")
                .next()
                .unwrap_or_default();
            Ok((name, serde_json::to_value(input)?))
        }
        Err(e) => {
            bail!(
                "Error decoding log for topic {}: {}",
                log_data.topics()[0],
                e
            );
        }
    }
}
