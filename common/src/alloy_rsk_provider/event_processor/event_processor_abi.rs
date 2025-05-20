use crate::types::{Address, RskEvent, RskLog};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::{Event, EventParam, JsonAbi};
use anyhow::{Context, Result, bail};
use hex;
use log::error;
use serde_json::{Value, json};
use std::str::FromStr;

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-133

pub fn process(
    contract_address: Address,
    rsk_log: RskLog,
    abi: &JsonAbi,
) -> Result<Option<RskEvent>> {
    if contract_address != rsk_log.info().address() {
        error!(
            "Log address {} does not match expected contract address {}",
            rsk_log.info().address(),
            contract_address
        );
        return Ok(None);
    }

    let event = abi.events.values().flatten().find(|e| {
        e.selector()
            .to_string()
            .eq_ignore_ascii_case(&rsk_log.event().topics()[0]) // TODO(Jira) another reason for https://rsklabs.atlassian.net/browse/UB-140
    });
    let event = event.unwrap();

    let mut decoded_log_input = serde_json::Map::new();

    let topic_params: Vec<&EventParam> = event.inputs.iter().filter(|i| i.indexed).collect();
    for (i, input) in topic_params.iter().enumerate() {
        let sol_type = DynSolType::from_str(input.ty.as_str())?;
        let sol_value = sol_type.abi_decode_params((&rsk_log.event().topics()[i]).as_ref())?;
        decoded_log_input.insert(input.name.to_string(), dyn_value_to_json(&sol_value)?);
    }

    let data_tuple = build_data_tuple(event, &rsk_log)?;

    let names_in_data: Vec<&EventParam> = event.inputs.iter().filter(|i| !i.indexed).collect();
    if let DynSolValue::Tuple(values) = data_tuple {
        for (i, input) in names_in_data.iter().enumerate() {
            let value = dyn_value_to_json(&values[i])?;
            decoded_log_input.insert(input.name.to_string(), value);
        }
    }

    let event_json = serde_json::to_value(event).context("Converting Alloy event to json")?;

    let rsk_event = RskEvent::new(event.name.to_string(), rsk_log.info().clone(), event_json);

    Ok(Some(rsk_event))
}

#[allow(unexpected_cfgs)]
fn dyn_value_to_json(value: &DynSolValue) -> Result<Value> {
    let parsed = match value {
        DynSolValue::Uint(num, _) => json!(format!("0x{:x}", num)),
        DynSolValue::Int(num, _) => json!(format!("0x{:x}", num)),
        DynSolValue::Bool(b) => json!(b),
        DynSolValue::String(s) => json!(s),
        DynSolValue::Address(addr) => json!(format!("{:?}", addr.to_string())),
        DynSolValue::FixedBytes(bytes, _) => json!(format!("0x{}", hex::encode(bytes))),
        DynSolValue::Bytes(bytes) => json!(format!("0x{}", hex::encode(bytes))),
        DynSolValue::Tuple(vals) | DynSolValue::Array(vals) | DynSolValue::FixedArray(vals) => {
            let mut result = Vec::new();
            for val in vals {
                result.push(dyn_value_to_json(val)?);
            }
            json!(result)
        }
        DynSolValue::Function(_) => {
            bail!("Unexpected Function on event");
        }
        #[cfg(feature = "eip712")]
        DynSolValue::CustomStruct(addr, ..) => {
            bail!("Unexpected CustomStruct on event");
        }
    };

    Ok(parsed)
}

fn build_data_tuple(event: &Event, rsk_log: &RskLog) -> Result<DynSolValue> {
    let data_types = event
        .inputs
        .iter()
        .filter(|i| !i.indexed)
        .flat_map(|i| DynSolType::from_str(i.ty.as_str()))
        .collect();

    let type_data_tuple = DynSolType::Tuple(data_types);

    // TODO(Jira) create custom types for topics, data... https://rsklabs.atlassian.net/browse/UB-140
    let data = &rsk_log.event().data().to_string();
    let data_as_hex =
        &hex::decode(&data.trim_start_matches("0x")).context("Decoding hex data tuple")?;
    let data_as_tuple = type_data_tuple
        .abi_decode_params(data_as_hex)
        .context(format!("Decoding tuple {type_data_tuple:?}"))?;
    Ok(data_as_tuple)
}
