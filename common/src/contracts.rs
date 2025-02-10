use anyhow::Result;
use std::fs;
use yaml_rust::{Yaml, YamlLoader};

#[derive(Debug)]
pub struct ContractInfo {
    pub address: String,
    pub name: String,
    pub abi: String,
}

// TODO(iago) think if still needed
pub fn get_managed_contracts_from_config_yaml(file_path: &str) -> Result<Vec<ContractInfo>> {
    let file_contents = fs::read_to_string(file_path).expect("Failed to read file content");
    let docs = YamlLoader::load_from_str(&file_contents).expect("Failed to parse YAML");
    let mut contracts: Vec<ContractInfo> = Vec::new();

    for doc in docs {
        let hash = doc.as_hash().expect("Failed to parse YAML");
        for (key, value) in hash {
            if let Yaml::String(name) = key {
                if let Yaml::Hash(fields) = value {
                    if let Some(Yaml::String(address)) =
                        fields.get(&Yaml::String("address".to_string()))
                    {
                        contracts.push(ContractInfo {
                            address: address.to_string(),
                            name: name.to_string(),
                            abi: format!("./abi/{}.json", address),
                        });
                    }
                }
            }
        }
    }

    println!("{:?}", contracts);

    Ok(contracts)
}
