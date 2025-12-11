use std::error::Error;
use std::str::FromStr;
use std::string::ToString;

use bitcoin::blockdata::block::Header;
use bitcoin::consensus::encode::deserialize as btc_deserialize;
use check_fork::BridgeEvent;
use check_fork::block_header::Block;
use primitive_types::{H256, U256};
use reqwest::Client;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Value, json};

const RSK_RPC_URL: &str = "https://public-node.rsk.co";

const SUPERBLOCK_THRESHOLD_FACTOR: u64 = 20;

// todo(fede) this should contain an RskBlockHeader
#[derive(Serialize, Deserialize, Debug, Clone)]
struct RskBlock {
    #[serde(deserialize_with = "parse_hex_to_u64", serialize_with = "parse_u64_to_hex")]
    number: u64,
    hash: H256,
    #[serde(rename = "parentHash")]
    parent: H256,
    #[serde(deserialize_with = "parse_rsk_difficulty")]
    difficulty: U256,
    #[serde(deserialize_with = "parse_hex_to_u64", serialize_with = "parse_u64_to_hex")]
    timestamp: u64,
    #[serde(
        rename = "bitcoinMergedMiningHeader",
        deserialize_with = "parse_bitcoin_header_to_pow"
    )]
    pow: H256,
    bridge_event: Option<BridgeEvent>,
    #[serde(default)]
    uncles: Vec<RskBlock>,
}

impl From<&RskBlock> for Block {
    fn from(rsk_block: &RskBlock) -> Self {
        Block {
            number: rsk_block.number,
            hash: rsk_block.hash,
            parent: rsk_block.parent,
            difficulty: rsk_block.difficulty,
            timestamp: rsk_block.timestamp,
            pow: rsk_block.pow,
            bridge_event: rsk_block.bridge_event.clone(),
            uncles: rsk_block.uncles.iter().map(Block::from).collect(),
        }
    }
}

/// Fetches blocks from RSK RPC endpoint.
///
/// # Errors
///
/// Returns an error if the HTTP request fails, JSON parsing fails, or block deserialization fails.
///
/// # Panics
///
/// This function may panic if `result.unwrap()` is called on a `None` value when processing block results.
pub async fn get_blocks(
    start_block_number: u64,
    num_of_blocks: u32,
    log_super_block: bool,
    has_bridge_event: bool,
) -> Result<Vec<Block>, Box<dyn Error>> {
    let client = Client::new();
    let mut blocks = vec![];

    for i in 0..num_of_blocks {
        let block_number_hex = format!("{:#x}", start_block_number + u64::from(i));
        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "eth_getBlockByNumber",
            "params": [block_number_hex, true],
            "id": 1,
        });

        let response = client.post(RSK_RPC_URL).json(&request_body).send().await?;
        let response_json: Value = response.json().await?;

        let error = response_json.get("error");
        let result = response_json.get("result");
        if error.is_some() {
            println!(
                "Error fetching block {}: {:?}",
                start_block_number + u64::from(i),
                response_json
            );
        } else if result.is_some() {
            // originally we had:
            // let block: RskBlock = serde_json::from_str(&result.unwrap().to_string())?;

            // remove next three lines when connection with check-fork is done and uncles come in right format
            let mut result = result.unwrap().clone();
            result["uncles"] = serde_json::Value::Array(Vec::new());
            let block: RskBlock = serde_json::from_str(&result.to_string())?;
            if log_super_block {
                log_if_superblock(&block)?;
            }

            blocks.push(block);
        }
    }

    // // Write blocks to the output file
    // let serialized_blocks = serde_json::to_string(&blocks)?;
    if has_bridge_event {
        let result: Vec<Block> = add_bridge_event(&blocks);
        Ok(result)
    } else {
        let result: Vec<Block> = blocks.iter().map(Block::from).collect();
        Ok(result)
    }
}

fn add_bridge_event(blocks: &[RskBlock]) -> Vec<Block> {
    blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut input_block = Block::from(b);
            if i == 0 {
                // TODO(Jira): https://rsklabs.atlassian.net/browse/UB-10
                input_block.bridge_event = Some(BridgeEvent {
                    utxo_id: "FAKE_UTXO_ID".to_string(),         // tmp
                    pegout_id: "FAKE_PEGOUT_ID".to_string(),     // tmp
                    operator_id: "FAKE_OPERATOR_ID".to_string(), // tmp
                });
            }
            input_block
        })
        .collect()
}

fn log_if_superblock(block: &RskBlock) -> Result<(), Box<dyn Error>> {
    // parse the block's actual PoW (from bitcoinMergedMiningHeader field) to decimal
    let actual_block_pow = U256::from_big_endian(block.pow.as_bytes());

    // compute the PoW target from difficulty by inversion
    // U256::MAX, the "difficulty 1" target, represents the easiest possible target
    // this conversion allows comparing target difficulty with the actual block PoW
    let target_block_pow =
        U256::MAX.checked_div(block.difficulty).ok_or("0 division on log_if_superblock")?;

    // define a superblock as one whose PoW is at least N times harder than the required target
    let superblock_pow = target_block_pow / SUPERBLOCK_THRESHOLD_FACTOR;

    // if the actual block PoW is lower (i.e., harder) than the SuperBlock threshold, we found a SuperBlock
    if actual_block_pow < superblock_pow {
        let timestamp_i64 = i64::try_from(block.timestamp).unwrap_or(i64::MAX);
        let formatted_time =
            chrono::DateTime::from_timestamp(timestamp_i64, 0).unwrap().format("%Y-%m-%d %H:%M:%S");

        println!(
            "SuperBlock: {}, pow: {:?}, threshold: 0x{:064x}, time: {}",
            block.number, &block.pow, superblock_pow, formatted_time
        );
    }

    Ok(())
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn parse_u64_to_hex<S>(v: &u64, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&format!("{v:#x}"))
}

fn parse_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: &str = Deserialize::deserialize(deserializer)?;
    u64::from_str_radix(hex.trim_start_matches("0x"), 16).map_err(de::Error::custom)
}

fn parse_rsk_difficulty<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let difficulty_hex: &str = Deserialize::deserialize(deserializer)?;
    let difficulty_dec = U256::from_str_radix(difficulty_hex, 16).map_err(de::Error::custom)?;

    Ok(difficulty_dec)
}

fn parse_bitcoin_header_to_pow<'de, D>(deserializer: D) -> Result<H256, D::Error>
where
    D: Deserializer<'de>,
{
    let hex = <&str>::deserialize(deserializer)?;
    let bytes = hex::decode(hex.trim_start_matches("0x")).map_err(de::Error::custom)?;
    // 80-byte → treat as full header, otherwise assume it is already a 32-byte hash
    if bytes.len() == 80 {
        btc_deserialize::<Header>(&bytes)
            .map(|h| H256::from_str(&h.block_hash().to_string()))
            .expect("Failed to deserialize hash")
            .map_err(de::Error::custom)
    } else {
        H256::from_str(hex).map_err(de::Error::custom)
    }
}
