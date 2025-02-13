use crate::event_processor::build_event_json;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::{Event, EventParam, JsonAbi};
use anyhow::{bail, Result};
use common::cache::LruCache;
use common::types::RskLog;
use hex;
use log::{debug, error};
use serde_json::{json, Value};
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::OnceLock;

static ABI_CACHE: OnceLock<LruCache<JsonAbi>> = OnceLock::new();

fn get_abi_cache() -> &'static LruCache<JsonAbi> {
    ABI_CACHE.get_or_init(|| LruCache::new(1000))
}

pub fn process(contract_address: &str, rsk_log: &RskLog, abi_path: &str) -> Result<Option<Value>> {
    if contract_address != rsk_log.data().address() {
        error!(
            "Log address {} does not match expected contract address {}",
            rsk_log.data().address(),
            contract_address
        );
        return Ok(None);
    }

    let json_abi = match get_abi_cache().get(&contract_address)? {
        Some(abi) => abi,
        None => {
            debug!("Adding ABI to cache: {}", abi_path);
            let abi_file = File::open(&abi_path)?;
            let abi_from_file = serde_json::from_reader(BufReader::new(abi_file))?;
            get_abi_cache().insert(&contract_address, &abi_from_file)?;
            abi_from_file
        }
    };

    let event = json_abi.events.values().flatten().find(|e| {
        e.selector()
            .to_string()
            .eq_ignore_ascii_case(&rsk_log.event().topics()[0]) // TODO(Jira) another reason for https://rsklabs.atlassian.net/browse/UB-43
    });
    let event = event.unwrap();

    let mut decoded_log_input = serde_json::Map::new();

    let topic_params: Vec<&EventParam> = event.inputs.iter().filter(|i| i.indexed).collect();
    for (i, input) in topic_params.iter().enumerate() {
        let sol_type = DynSolType::from_str(input.ty.as_str())?;
        let sol_value = sol_type.abi_decode_params((&rsk_log.event().topics()[i]).as_ref())?;
        decoded_log_input.insert(input.name.to_string(), dyn_value_to_json(&sol_value)?);
    }

    let data_tuple = build_data_tuple(event, &rsk_log.event().data().to_string())?;
    let names_in_data: Vec<&EventParam> = event.inputs.iter().filter(|i| !i.indexed).collect();
    if let DynSolValue::Tuple(values) = data_tuple {
        for (i, input) in names_in_data.iter().enumerate() {
            decoded_log_input.insert(input.name.to_string(), dyn_value_to_json(&values[i])?);
        }
    }

    let event_json = build_event_json(&event.name, &rsk_log, decoded_log_input.into())?;
    Ok(Some(event_json))
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

    // TODO(Jira) create custom types for topics, data... https://rsklabs.atlassian.net/browse/UB-43
    let data_as_hex = &hex::decode(&data.trim_start_matches("0x"))?;
    let type_data_tuple = DynSolType::Tuple(data_types);
    let data_as_tuple = type_data_tuple.abi_decode_params(data_as_hex)?;
    Ok(data_as_tuple)
}
