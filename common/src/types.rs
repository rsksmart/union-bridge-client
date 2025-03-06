use crate::errors;
use alloy_json_abi::JsonAbi;
use bitcoin::{blockdata::block::Header, consensus::encode::deserialize as btc_deserialize};
use primitive_types::{H256, U256};
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fmt,
    ops::{Add, Sub},
    string::ToString,
};

//// Represents a rootstock block hash.
///
/// This is a wrapper around [`H256`] to enforce type safety.
///
/// # Examples
///
/// ```
/// use primitive_types::H256;
/// use common::types::BlockHash;
///
/// let raw_hash = H256::random();
/// let block_hash = BlockHash::from(raw_hash);
///
/// println!("Block hash: {}", block_hash);
/// ```
#[derive(Serialize, Deserialize, Copy, Debug, PartialEq, Clone)]
pub struct BlockHash(H256);

impl BlockHash {
    pub fn value(self) -> H256 {
        self.0
    }
}

impl From<H256> for BlockHash {
    fn from(h256: H256) -> Self {
        Self(h256)
    }
}

impl TryFrom<&str> for BlockHash {
    type Error = errors::BlockHashError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let value = value.trim_start_matches("0x");
        let bytes = hex::decode(value)?;
        let h256 = H256::from_slice(&bytes);

        Ok(Self(h256))
    }
}

impl fmt::Display for BlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

/// Represents a block number in the rootstock blockchain.
///
/// Block numbers are typically represented as 64-bit unsigned integers (`u64`).
///
/// This struct ensures type safety when working with block numbers, preventing
/// accidental misuse of raw `u64` values in places where a `BlockNumber` is expected.
///
/// # Example
///
/// ```
/// use common::types::BlockNumber;
///
/// let block_100 = BlockNumber::from(100);
/// let next_block = block_100 + 1;
///
/// assert_eq!(next_block, BlockNumber::from(101));
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub struct BlockNumber(u64);

impl BlockNumber {
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl From<u64> for BlockNumber {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Add<u64> for BlockNumber {
    type Output = Self;

    fn add(self, rhs: u64) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub<u64> for BlockNumber {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self {
        Self(self.0 - rhs)
    }
}

impl PartialEq<u64> for BlockNumber {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<u64> for BlockNumber {
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        Some(self.0.cmp(other))
    }
}

impl fmt::Display for BlockNumber {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a block timestamp in the rootstock blockchain.
///
/// Block timestamps are typically represented as 64-bit unsigned integers (`u64`).
///
/// This struct ensures type safety when working with block timestamps, preventing
/// accidental misuse of raw `u64` values in places where a `BlockTimestamp` is expected.
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq, Copy, Clone)]
pub struct BlockTimestamp(u64);

impl BlockTimestamp {
    pub fn value(self) -> u64 {
        self.0
    }
}

impl From<u64> for BlockTimestamp {
    fn from(timestamp: u64) -> Self {
        Self(timestamp)
    }
}

impl Add<u64> for BlockTimestamp {
    type Output = Self;

    fn add(self, rhs: u64) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub<u64> for BlockTimestamp {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self {
        Self(self.0 - rhs)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct RskBlock {
    number: BlockNumber,
    hash: BlockHash,
    parent_hash: BlockHash,
    difficulty: U256,
    timestamp: BlockTimestamp,
    total_difficulty: U256,
    pow: String,
}

impl From<RskRpcBlock> for RskBlock {
    fn from(rpc_block: RskRpcBlock) -> Self {
        Self::new(
            rpc_block.number,
            rpc_block.hash,
            rpc_block.parent_hash,
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
        hash: BlockHash,
        parent_hash: BlockHash,
        difficulty: U256,
        timestamp: BlockTimestamp,
        pow: String,
        total_difficulty: U256,
    ) -> Self {
        RskBlock {
            number,
            hash,
            parent_hash,
            difficulty,
            timestamp,
            pow,
            total_difficulty,
        }
    }

    pub fn number(&self) -> BlockNumber {
        self.number
    }

    pub fn hash(&self) -> BlockHash {
        self.hash
    }

    pub fn parent_hash(&self) -> BlockHash {
        self.parent_hash
    }

    pub fn difficulty(&self) -> U256 {
        self.difficulty
    }

    pub fn timestamp(&self) -> BlockTimestamp {
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
    #[serde(deserialize_with = "parse_hex_to_block_number")]
    number: BlockNumber,
    #[serde(deserialize_with = "parse_hex_to_block_hash")]
    hash: BlockHash,
    #[serde(rename = "parentHash", deserialize_with = "parse_hex_to_block_hash")]
    parent_hash: BlockHash,
    #[serde(deserialize_with = "parse_rsk_difficulty")]
    difficulty: U256,
    #[serde(deserialize_with = "parse_hex_to_block_timestamp")]
    timestamp: BlockTimestamp,
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

fn parse_hex_to_block_number<'de, D>(deserializer: D) -> Result<BlockNumber, D::Error>
where
    D: Deserializer<'de>,
{
    parse_hex_to_u64(deserializer).map(BlockNumber::from)
}

fn parse_hex_to_block_timestamp<'de, D>(deserializer: D) -> Result<BlockTimestamp, D::Error>
where
    D: Deserializer<'de>,
{
    parse_hex_to_u64(deserializer).map(BlockTimestamp::from)
}

fn parse_hex_to_block_hash<'de, D>(deserializer: D) -> Result<BlockHash, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;

    BlockHash::try_from(hex.as_str()).map_err(|err| {
        de::Error::custom(format!(
            "Failed to parse hex to block hash: {} - {}",
            hex, err
        ))
    })
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
    block_hash: BlockHash,
    number: BlockNumber,
    tx_hash: String,
    log_index: u64,
    removed: bool,
}

impl LogInfo {
    pub fn new(
        address: String,
        block_hash: BlockHash,
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

    pub fn block_hash(&self) -> BlockHash {
        self.block_hash
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

#[cfg(test)]
mod tests {
    use crate::errors::BlockHashError;
    use crate::types::BlockHash;
    use test_utils::rsk_entity_generator::DEFAULT_BLOCK_HASH;

    #[test]
    fn test_valid_block_hash_when_valid_hash_is_provided_should_return_ok() {
        let block_hash = BlockHash::try_from(DEFAULT_BLOCK_HASH);

        assert!(block_hash.is_ok());
    }

    #[test]
    fn test_invalid_block_hash_when_invalid_hash_is_provided_should_return_error() {
        let invalid_hash = "0xinvalidhex";
        let block_hash = BlockHash::try_from(invalid_hash);

        assert!(block_hash.is_err());

        if let Err(BlockHashError::InvalidHex(_)) = block_hash {
            // The error was expected due to invalid hex input
        } else {
            panic!(
                "Expected BlockHashError::InvalidHex, but got: {:?}",
                block_hash
            );
        }
    }

    #[test]
    fn test_missing_prefix_when_hash_without_prefix_is_provided_should_return_ok() {
        let valid_hash_without_prefix =
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let block_hash = BlockHash::try_from(valid_hash_without_prefix);

        assert!(block_hash.is_ok());
    }
}
