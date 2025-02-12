use alloy_primitives::{hex, LogData};
use alloy_sol_types::private::B256;
use alloy_sol_types::{sol, SolEvent};
use anyhow::{bail, Result};
use common::types::RskLog;
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::ops::Deref;

// define here known events, they are converted to Rust structs (mapped types, etc.)
sol! {
    #[derive(Serialize, Deserialize, Debug)]
    event ValueUpdate(
        uint256 value,
        bytes32 dataFeedId,
        uint256 updatedAt
    );
}

pub fn process(log: &RskLog) -> Result<Option<impl Serialize>> {
    let parsed_topics: Vec<B256> = log
        .topics
        .iter()
        .filter_map(|topic| topic.parse::<B256>().ok())
        .collect();

    let log_data = LogData::new(parsed_topics, hex::decode(&log.data)?.into());
    if log_data.is_none() {
        error!("Failed to parse log data: {:?}", log_data);
    }
    let log_data = log_data.unwrap();

    let topics0 = &log.topics[0];
    if topics0 == ValueUpdate::SIGNATURE_HASH.to_string().deref() {
        Ok(Some(decode_value_update(&log_data)?))
    }
    // other types here in the future
    else {
        Ok(None)
    }
}

fn decode_value_update(log_data: &LogData) -> Result<impl Serialize> {
    let decoded_log = ValueUpdate::decode_log_data(&log_data, true);

    match decoded_log {
        Ok(t) => {
            debug!("Decoded ValueUpdate: {:?}", t);
            Ok(t)
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
