use crate::types::{RskEvent, RskLog};
use alloy_primitives::LogData;
use alloy_sol_types::private::B256;
use alloy_sol_types::{SolEvent, sol};
use anyhow::{Context, Result, bail};
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

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-133

pub fn process(rsk_log: RskLog) -> Result<Option<RskEvent>> {
    let parsed_topics: Vec<B256> = rsk_log
        .event()
        .topics()
        .iter()
        .map(|topic| B256::from(*topic))
        .collect();

    let data = rsk_log.event().data().as_bytes().to_vec();

    let log_data = LogData::new(parsed_topics, data.into());
    if log_data.is_none() {
        bail!("Failed to create Alloy LogData from rsk_log")
    }
    let log_data = log_data.unwrap();

    let topic0 = log_data.topics().get(0);
    let event_name_and_input = match topic0 {
        Some(ev) if *ev == ValueUpdate::SIGNATURE_HASH => {
            Some(decode_event_input::<ValueUpdate>(&log_data)?)
        }
        Some(ev) if *ev == LogValue::SIGNATURE_HASH => {
            Some(decode_event_input::<LogValue>(&log_data)?)
        }
        // other types here in the future
        _ => None,
    };

    if event_name_and_input.is_none() {
        return Ok(None);
    }

    let (name, decoded_log_input) = event_name_and_input.unwrap();

    let event = RskEvent::new(name.to_string(), rsk_log.info().clone(), decoded_log_input);
    Ok(Some(event))
}

fn decode_event_input<T: SolEvent + Serialize + Debug>(
    log_data: &LogData,
) -> Result<(&str, Value)> {
    let name = std::any::type_name::<T>()
        .rsplit("::")
        .next()
        .unwrap_or_default();

    let decoded_event = T::decode_log_data(&log_data, true)?;
    let event_json = serde_json::to_value(&decoded_event)
        .context(format!("Failed to serialize {name:?} to json"))?;
    Ok((name, event_json))
}
