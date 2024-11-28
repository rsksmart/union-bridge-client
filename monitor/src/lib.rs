use bitcoin::blockdata::block::Header;
use bitcoin::consensus::encode::deserialize as btc_deserialize;
use check_fork::{Block, BridgeEvent};
use primitive_types::U256;
use reqwest::Client;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::string::ToString;

const RSK_RPC_URL: &str = "https://public-node.rsk.co";

#[derive(Serialize, Deserialize, Debug)]
struct RskBlock {
    #[serde(deserialize_with = "hex_to_u32")]
    number: u32,
    hash: String,
    #[serde(rename = "parentHash")]
    parent: String,
    #[serde(deserialize_with = "parse_rsk_difficulty")]
    difficulty: U256,
    #[serde(deserialize_with = "parse_hex_to_u64")]
    timestamp: u64,
    #[serde(
        rename = "bitcoinMergedMiningHeader",
        deserialize_with = "parse_bitcoin_header_to_pow"
    )]
    pow: String,
    // bridge_event: Option<BridgeEvent>, // TODO implement
    // uncles: Vec<Block>, // TODO test with some
}

impl From<&RskBlock> for Block {
    fn from(rsk_block: &RskBlock) -> Self {
        Block {
            number: rsk_block.number,
            hash: rsk_block.hash.clone(),
            parent: rsk_block.parent.clone(),
            difficulty: rsk_block.difficulty,
            timestamp: rsk_block.timestamp,
            pow: rsk_block.pow.clone(),
            bridge_event: None, // TODO remove this hardcoding
            uncles: vec![],     // TODO test with some
        }
    }
}

pub async fn get_blocks(
    start_block_number: u32,
    num_of_blocks: u16,
) -> Result<Vec<Block>, Box<dyn Error>> {
    let client = Client::new();

    let mut blocks = vec![];

    for i in 0..num_of_blocks {
        let block_number_hex = format!("0x{:x}", start_block_number + i as u32);
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
                start_block_number - i as u32,
                response_json
            );
        } else if result.is_some() {
            let block: RskBlock = serde_json::from_str(&result.unwrap().to_string())?;

            let pow_dec =
                U256::from_str_radix(&block.pow, 16).map_err(|_| "Failed to parse block PoW")?;
            let effort = U256::MAX / pow_dec;

            log_if_superblock(&block, effort);

            blocks.push(block);
        }
    }

    let result: Vec<Block> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut input_block = Block::from(b);
            if i == 0 {
                // TODO tmp fake event data
                input_block.bridge_event = Some(BridgeEvent {
                    utxo_id: "FAKE_UTXO_ID".to_string(),         // tmp
                    operator_id: "FAKE_OPERATOR_ID".to_string(), // tmp
                });
            }
            input_block
        })
        .collect();

    println!("get_blocks done, total blocks '{}'", result.len());

    Ok(result)
}

fn log_if_superblock(block: &RskBlock, effort: U256) {
    let superblock_difficulty = block.difficulty * 20;
    let superblock_effort = U256::MAX / superblock_difficulty;
    if effort >= superblock_difficulty {
        println!(
            "SuperBlock: {}, pow: {}, effort: {:064x}, time: {}",
            block.number,
            &block.pow,
            superblock_effort,
            chrono::DateTime::from_timestamp(block.timestamp as i64, 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
        );
    }
}

fn hex_to_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let hex_str: &str = Deserialize::deserialize(deserializer)?;
    u32::from_str_radix(hex_str.trim_start_matches("0x"), 16).map_err(de::Error::custom)
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
    let difficulty_dec = U256::from_str_radix(&difficulty_hex, 16).map_err(de::Error::custom)?;

    Ok(difficulty_dec)
}

fn parse_bitcoin_header_to_pow<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let header_hex: &str = Deserialize::deserialize(deserializer)?;
    let header_bytes =
        hex::decode(header_hex.trim_start_matches("0x")).map_err(de::Error::custom)?;

    // deserialize the header bytes into a Bitcoin Header and extract the hash
    let header_hash = btc_deserialize(&header_bytes)
        .map(|h: Header| h.block_hash().to_string())
        .map_err(de::Error::custom)?;

    // dbg!((header_hex, header_hash));

    Ok(header_hash)
}