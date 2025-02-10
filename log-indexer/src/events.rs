use alloy_primitives::{hex, LogData, U256};
use alloy_sol_types::private::B256;
use alloy_sol_types::{sol, SolEvent};
use anyhow::{anyhow, Result};
use log::{debug, error};
use serde::{Deserialize, Serialize, Serializer};

sol! {
    #[derive(Serialize, Deserialize, Debug)]
    event ValueUpdate(
        uint256 value,
        bytes32 dataFeedId,
        uint256 updatedAt
    );
}

pub fn parse_event_to_json(topics: Vec<String>, data: String) -> Result<String> {
    let parsed_topics: Vec<B256> = topics
        .iter()
        .filter_map(|topic| topic.parse::<B256>().ok())
        .collect();

    let log_data = LogData::new(parsed_topics, hex::decode(data)?.into());
    if log_data.is_none() {
        error!("Failed to parse log data: {:?}", log_data);
    }

    if ValueUpdate::SIGNATURE_HASH.to_string() == topics[0] {
        let test = ValueUpdate::decode_log_data(&log_data.unwrap(), true);
        match test {
            Ok(t) => {
                debug!("Decoded: {:?}", t);
                return Ok(serde_json::to_string(&t)?);
            }
            Err(e) => {
                error!(
                    "Error decoding: {:?}, signature: {:?}",
                    e,
                    ValueUpdate::SIGNATURE_HASH.to_string()
                );
            }
        }
    } else {
        error!(
            "Unknown event type: {:?}, log_data: {:?}, signature: {:?}",
            topics[0],
            log_data,
            ValueUpdate::SIGNATURE_HASH.to_string()
        );
    }

    Ok("NONE".to_string())
}
