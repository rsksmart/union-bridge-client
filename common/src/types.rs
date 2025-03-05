use alloy_json_abi::JsonAbi;
use bitcoin::{blockdata::block::Header, consensus::encode::deserialize as btc_deserialize};
use primitive_types::U256;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::string::ToString;

/// Represents a block number in the blockchain.
///
/// This is an alias for `u64`, as block numbers are typically 64-bit unsigned integers.
pub type BlockNumber = u64;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct RskBlock {
    number: BlockNumber,
    hash: String,
    parent: String,
    difficulty: U256,
    timestamp: u64,
    total_difficulty: U256,
    pow: String,
}

impl From<RskRpcBlock> for RskBlock {
    fn from(rpc_block: RskRpcBlock) -> Self {
        Self::new(
            rpc_block.number,
            rpc_block.hash,
            rpc_block.parent,
            rpc_block.difficulty,
            rpc_block.timestamp,
            rpc_block.pow,
            rpc_block.total_difficulty,
        )
    }
}

impl RskBlock {
    pub fn new(
        number: BlockNumber,
        hash: String,
        parent: String,
        difficulty: U256,
        timestamp: u64,
        pow: String,
        total_difficulty: U256,
    ) -> Self {
        RskBlock {
            number,
            hash,
            parent,
            difficulty,
            timestamp,
            pow,
            total_difficulty,
        }
    }

    pub fn number(&self) -> BlockNumber {
        self.number
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn parent(&self) -> &str {
        &self.parent
    }

    pub fn difficulty(&self) -> U256 {
        self.difficulty
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn pow(&self) -> &str {
        &self.pow
    }

    pub fn total_difficulty(&self) -> U256 {
        self.total_difficulty
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RskRpcBlock {
    #[serde(deserialize_with = "parse_hex_to_u64")]
    number: BlockNumber,
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
    #[serde(deserialize_with = "parse_rsk_difficulty", rename = "totalDifficulty")]
    total_difficulty: U256,
}

fn parse_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;
    u64::from_str_radix(hex.trim_start_matches("0x"), 16).map_err(de::Error::custom)
}

fn parse_rsk_difficulty<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let difficulty_hex: String = Deserialize::deserialize(deserializer)?;
    let difficulty_dec = U256::from_str_radix(&difficulty_hex, 16).map_err(de::Error::custom)?;

    Ok(difficulty_dec)
}

fn parse_bitcoin_header_to_pow<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let header_hex: String = Deserialize::deserialize(deserializer)?;
    let header_bytes =
        hex::decode(header_hex.trim_start_matches("0x")).map_err(de::Error::custom)?;

    // deserialize the header bytes into a Bitcoin Header and extract the hash
    let header_hash = btc_deserialize(&header_bytes)
        .map(|h: Header| h.block_hash().to_string())
        .map_err(de::Error::custom)?;

    Ok(header_hash)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-43
pub struct RskLog {
    info: LogInfo,
    event: LogEvent,
}

impl RskLog {
    pub fn new(data: LogInfo, event: LogEvent) -> Self {
        Self { info: data, event }
    }

    pub fn info(&self) -> &LogInfo {
        &self.info
    }

    pub fn event(&self) -> &LogEvent {
        &self.event
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RskEvent {
    name: String,
    info: LogInfo,
    input: Value,
}

impl RskEvent {
    pub fn new(name: String, info: LogInfo, input: Value) -> Self {
        Self { name, info, input }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn info(&self) -> &LogInfo {
        &self.info
    }

    pub fn input(&self) -> &Value {
        &self.input
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogInfo {
    address: String,
    block_hash: String,
    number: BlockNumber,
    tx_hash: String,
    log_index: u64,
    removed: bool,
}

impl LogInfo {
    pub fn new(
        address: String,
        block_hash: String,
        number: BlockNumber,
        tx_hash: String,
        log_index: u64,
        removed: bool,
    ) -> Self {
        LogInfo {
            address,
            block_hash,
            number,
            tx_hash,
            log_index,
            removed,
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    pub fn number(&self) -> BlockNumber {
        self.number
    }

    pub fn tx_hash(&self) -> &str {
        &self.tx_hash
    }

    pub fn log_index(&self) -> u64 {
        self.log_index
    }

    pub fn removed(&self) -> bool {
        self.removed
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogEvent {
    data: String,
    topics: Vec<String>,
}

impl LogEvent {
    pub fn new(data: String, topics: Vec<String>) -> Self {
        LogEvent { data, topics }
    }

    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn topics(&self) -> &Vec<String> {
        &self.topics
    }
}

#[derive(Debug, Clone)]
pub struct ContractInfo {
    pub address: String,
    pub name: String,
    pub abi: Option<JsonAbi>,
}
