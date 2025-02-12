use crate::event_processor::managed_contracts::ContractInfo;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::{Event, EventParam};
use anyhow::{bail, Result};
use common::types::RskLog;
use hex;
use log::error;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;

pub fn process(log: &RskLog, contract: &ContractInfo) -> Result<Option<impl Serialize>> {
    if contract.address != log.address {
        error!(
            "Log address {} does not match expected contract address {}",
            log.address, contract.address
        );
        return Ok(None);
    }

    let event = contract.abi.events.values().flatten().find(|e| {
        e.selector()
            .to_string()
            .eq_ignore_ascii_case(&log.topics[0]) // TODO(iago) due to this, I think it's better to have a custom type for Address
    });
    let event = event.unwrap();

    let mut decoded_log: HashMap<String, Value> = HashMap::new();

    decoded_log.insert("name".to_string(), json!(event.name));
    decoded_log.insert("address".to_string(), json!(log.address));

    let topic_params: Vec<&EventParam> = event.inputs.iter().filter(|i| i.indexed).collect();
    for (i, input) in topic_params.iter().enumerate() {
        let sol_type = DynSolType::from_str(input.ty.as_str())?;
        let sol_value = sol_type.abi_decode_params((&log.topics[i]).as_ref())?;
        decoded_log.insert(input.name.to_string(), dyn_value_to_json(&sol_value)?);
    }

    let data_tuple = build_data_tuple(event, &log.data)?;
    let names_in_data: Vec<&EventParam> = event.inputs.iter().filter(|i| !i.indexed).collect();
    if let DynSolValue::Tuple(values) = data_tuple {
        for (i, input) in names_in_data.iter().enumerate() {
            decoded_log.insert(input.name.to_string(), dyn_value_to_json(&values[i])?);
        }
    }

    Ok(Some(decoded_log))
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

fn build_data_tuple(event: &Event, data: &String) -> Result<DynSolValue> {
    let data_types = event
        .inputs
        .iter()
        .filter(|i| !i.indexed)
        .flat_map(|i| DynSolType::from_str(i.ty.as_str()))
        .collect();

    // TODO(iago) probably it makes no sense to make data a string on the provider to then undo the mapping
    let data_as_hex = &hex::decode(&data.trim_start_matches("0x"))?;
    let type_data_tuple = DynSolType::Tuple(data_types);
    let data_as_tuple = type_data_tuple.abi_decode_params(data_as_hex)?;
    Ok(data_as_tuple)
}
