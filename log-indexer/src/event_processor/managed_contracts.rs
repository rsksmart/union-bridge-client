use alloy_json_abi::JsonAbi;
use anyhow::Result;
use log::debug;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use yaml_rust::{Yaml, YamlLoader};

#[derive(Debug)]
pub struct ContractInfo {
    pub address: String,
    pub name: String,
    pub abi: JsonAbi,
}

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
                        if let Some(Yaml::String(abi)) =
                            fields.get(&Yaml::String("abi".to_string()))
                        {
                            let abi_path = format!("{}/abi/{}", file_path, abi);
                            let file = File::open(abi_path)?;
                            let reader = BufReader::new(file);
                            let json_abi: JsonAbi = serde_json::from_reader(reader)?;

                            contracts.insert(
                                address.to_string(),
                                ContractInfo {
                                    address: address.to_string(),
                                    name: name.to_string(),
                                    abi: json_abi,
                                },
                            );
                        }
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
