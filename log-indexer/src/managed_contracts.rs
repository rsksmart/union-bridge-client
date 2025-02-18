use anyhow::Result;
use common::types::ContractInfo;
use log::debug;
use std::collections::HashMap;
use std::fs;
use yaml_rust::yaml::Hash;
use yaml_rust::{Yaml, YamlLoader};

pub fn load_managed_contracts_from_config(
    file_path: &str,
) -> Result<HashMap<String, ContractInfo>> {
    let yaml_path = format!("{}/managed_contracts.yaml", file_path);
    let file_contents = fs::read_to_string(&yaml_path).expect("Failed to read file content");
    let docs = YamlLoader::load_from_str(&file_contents).expect("Failed to parse YAML");

    let contracts = docs
        .iter()
        .filter_map(Yaml::as_hash) // Extract top-level hash
        .flat_map(|hash| hash.iter())
        .filter_map(|(key, value)| parse_contract_info(key.as_str()?, value.as_hash()?, file_path))
        .collect::<HashMap<_, _>>();

    debug!(
        "Managed contracts: {:?}",
        contracts
            .iter()
            .map(|(address, contract)| format!("{} - {}", contract.name, address))
            .collect::<Vec<_>>()
    );

    Ok(contracts)
}

fn parse_contract_info(
    name: &str,
    fields: &Hash,
    abi_path: &str,
) -> Option<(String, ContractInfo)> {
    let address = fields
        .get(&Yaml::String("address".to_string()))?
        .as_str()?
        .to_string();
    let abi = fields
        .get(&Yaml::String("abi".to_string()))
        .and_then(Yaml::as_str)
        .map(|abi| format!("{}/abi/{}", abi_path, abi));

    Some((
        address.clone(),
        ContractInfo {
            address,
            name: name.to_string(),
            abi_file: abi,
        },
    ))
}
