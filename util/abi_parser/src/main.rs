use alloy_json_abi::JsonAbi;
use alloy_primitives::{Bytes, LogData, B256, U256};
use alloy_sol_types::{sol, SolEvent, SolType};
use anyhow::Result;
use log::debug;
use serde::{Deserialize, Serialize, Serializer};
use std::fs::File;
use std::io::BufReader;

fn serialize_u256_as_decimal<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    debug!("======={}", &value.to_string());
    serializer.serialize_str(&value.to_string())
}

sol! {
    #[derive(Serialize, Deserialize, Debug)]
    event ValueUpdate(
        #[serde(serialize_with = "serialize_u256_as_decimal")] uint256 value,
        bytes32 dataFeedId,
        #[serde(serialize_with = "serialize_u256_as_decimal")] uint256 updatedAt
    );
}

fn main() -> Result<()> {
    let abi = "/Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/log-indexer/config/abi/0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761.json";

    let file = File::open(abi)?;
    let reader = BufReader::new(file);
    let json_abi: JsonAbi = serde_json::from_reader(reader)?;

    let address = "0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761";
    let topic0 = "0xf36866d965ee70c8632ff558f5cf8d41ee9ca1d0d0bc7700786e57be60747390";
    let topics = vec![topic0.to_string()];
    let parsed_topics: Vec<B256> = topics
        .iter()
        .filter_map(|topic| topic.parse::<B256>().ok())
        .collect();

    let data: Vec<u8> = hex::decode("000000000000000000000000000000000000000000000000000000000053ff5852494600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000067ac7f97")?;

    let test = LogData::new(parsed_topics, Bytes::from(data));

    let test = ValueUpdate::decode_log_data(&test.unwrap(), true)?;
    let aaa = test.value;

    let test = serde_json::to_string(&test)?;

    println!("{:?}", test);

    Ok(())
}
