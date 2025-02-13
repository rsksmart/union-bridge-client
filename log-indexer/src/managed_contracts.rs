use anyhow::Result;
use common::types::ContractInfo;
use log::debug;
use std::collections::HashMap;
use std::fs;
use yaml_rust::{Yaml, YamlLoader};

pub fn load_managed_contracts_from_config(
    file_path: &str,
) -> Result<HashMap<String, ContractInfo>> {
    let yaml_path = format!("{}/managed_contracts.yaml", file_path);
    let file_contents = fs::read_to_string(yaml_path).expect("Failed to read file content");
    let docs = YamlLoader::load_from_str(&file_contents).expect("Failed to parse YAML");
    let mut contracts: HashMap<String, ContractInfo> = HashMap::new();

    for doc in docs {
        let hash = doc.as_hash().expect("Failed to parse YAML");
        for (key, value) in hash {
            if let Yaml::String(name) = key {
                if let Yaml::Hash(fields) = value {
                    if let Some(Yaml::String(address)) =
                        fields.get(&Yaml::String("address".to_string()))
                    {
                        let abi = fields
                            .get(&Yaml::String("abi".to_string()))
                            .and_then(|v| v.as_str())
                            .map(|abi| format!("{}/abi/{}", file_path, abi));
                        contracts.insert(
                            address.to_string(),
                            ContractInfo {
                                address: address.to_string(),
                                name: name.to_string(),
                                abi_file: abi,
                            },
                        );
                    }
                }
            }
        }
    }

    debug!(
        "Managed contracts: {:?}",
        contracts
            .iter()
            .map(|c| format!("{} - {}", &c.1.name, c.0))
            .collect::<Vec<_>>()
    );

    Ok(contracts)
}
