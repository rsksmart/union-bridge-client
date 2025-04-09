use alloy_json_abi::JsonAbi;
use bitcoin::{blockdata::block::Header, consensus::encode::deserialize as btc_deserialize};
use primitive_types::{H160, H256, U256};
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fmt,
    ops::{Add, Mul, Sub},
    str::FromStr,
    string::ToString,
};

//// Represents a rootstock block hash.
///
/// This struct ensures type safety when working with block hashes, preventing
/// accidental misuse of raw `H256` values in places where a `BlockHash` is expected.
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
    type Error = hex::FromHexError;

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

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl Sub<u64> for BlockNumber {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self::Output {
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

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl Sub<u64> for BlockTimestamp {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self::Output {
        Self(self.0 - rhs)
    }
}

/// Represents the block difficulty for a block in the rootstock blockchain.
///
/// This is a wrapper around [`U256`] to enforce type safety.
///
/// This struct ensures type safety when working with block difficulty, preventing
/// accidental misuse of raw `U256` values in places where a `BlockDifficulty` is expected.
///
/// # Example
///
/// ```
/// use primitive_types::U256;
/// use common::types::BlockDifficulty;
///
/// let value = U256::from(10);
/// let block_difficulty = BlockDifficulty::from(value);
///
/// println!("Block difficulty: {}", block_difficulty);
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq, PartialOrd, Copy, Clone)]
pub struct BlockDifficulty(U256);

impl BlockDifficulty {
    pub fn value(self) -> U256 {
        self.0
    }
}

impl From<U256> for BlockDifficulty {
    fn from(u256: U256) -> Self {
        Self(u256)
    }
}

impl Add for BlockDifficulty {
    type Output = BlockDifficulty;

    fn add(self, rhs: BlockDifficulty) -> Self::Output {
        BlockDifficulty(self.0 + rhs.0)
    }
}

impl Sub for BlockDifficulty {
    type Output = BlockDifficulty;

    fn sub(self, rhs: BlockDifficulty) -> Self::Output {
        BlockDifficulty(self.0 - rhs.0)
    }
}

impl Mul for BlockDifficulty {
    type Output = BlockDifficulty;

    fn mul(self, rhs: BlockDifficulty) -> Self::Output {
        BlockDifficulty(self.0 * rhs.0)
    }
}

impl fmt::Display for BlockDifficulty {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents the block hash of the Bitcoin merged mining header as
/// proof of work for a given Rootstock block.
///
/// This struct ensures type safety, preventing accidental misuse of raw `H256`
/// values in places where a `BlockPow` is expected.
///
/// This is a wrapper around [`H256`] to enforce type safety.
///
/// # Examples
///
/// ```
/// use primitive_types::H256;
/// use common::types::BlockPow;
///
/// let value = H256::random();
/// let pow = BlockPow::from(value);
///
/// println!("Block PoW: {}", pow);
/// ```
#[derive(Serialize, Deserialize, Copy, Debug, PartialEq, Clone)]
pub struct BlockPow(H256);

impl BlockPow {
    pub fn value(self) -> H256 {
        self.0
    }
}

impl From<H256> for BlockPow {
    fn from(h256: H256) -> Self {
        Self(h256)
    }
}

impl TryFrom<&str> for BlockPow {
    type Error = Box<dyn std::error::Error>;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let value = value.trim_start_matches("0x");
        let header_bytes = hex::decode(value)?;
        let header_hash = btc_deserialize::<Header>(&header_bytes)?
            .block_hash()
            .to_string();
        let h256 = H256::from_str(header_hash.as_str())?;

        Ok(Self(h256))
    }
}

impl fmt::Display for BlockPow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

/// Represents a Rootstock address.
///
/// This struct enforces type safety when working with addresses,
/// preventing accidental misuse of raw `H160` values.
///
/// # Examples
///
/// ```
/// use primitive_types::H160;
/// use common::types::Address;
///
/// let raw_address = H160::random();
/// let address = Address::from(raw_address);
///
/// println!("Address: {}", address);
/// ```
#[derive(Serialize, Deserialize, Copy, Debug, Ord, PartialOrd, PartialEq, Eq, Clone, Hash)]
pub struct Address(H160);

impl Address {
    pub fn value(self) -> H160 {
        self.0
    }
}

impl From<H160> for Address {
    fn from(h160: H160) -> Self {
        Self(h160)
    }
}

impl TryFrom<&str> for Address {
    type Error = hex::FromHexError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let value = value.trim_start_matches("0x");
        let bytes = hex::decode(value)?;
        let h160 = H160::from_slice(&bytes);

        Ok(Self(h160))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct RskBlock {
    number: BlockNumber,
    hash: BlockHash,
    parent_hash: BlockHash,
    timestamp: BlockTimestamp,
    difficulty: BlockDifficulty,
    total_difficulty: BlockDifficulty,
    pow: BlockPow,
    uncles: Vec<BlockHash>,
}

impl RskBlock {
    pub fn new(
        number: BlockNumber,
        hash: BlockHash,
        parent_hash: BlockHash,
        timestamp: BlockTimestamp,
        difficulty: BlockDifficulty,
        total_difficulty: BlockDifficulty,
        pow: BlockPow,
        uncles: Vec<BlockHash>,
    ) -> Self {
        RskBlock {
            number,
            hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            pow,
            uncles,
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

    pub fn timestamp(&self) -> BlockTimestamp {
        self.timestamp
    }

    pub fn difficulty(&self) -> BlockDifficulty {
        self.difficulty
    }

    pub fn total_difficulty(&self) -> BlockDifficulty {
        self.total_difficulty
    }

    pub fn pow(&self) -> BlockPow {
        self.pow
    }

    pub fn uncles(&self) -> Vec<BlockHash> {
        self.uncles.clone()
    }
}

impl From<RskRpcBlock> for RskBlock {
    fn from(rpc_block: RskRpcBlock) -> Self {
        Self::new(
            rpc_block.number,
            rpc_block.hash,
            rpc_block.parent_hash,
            rpc_block.timestamp,
            rpc_block.difficulty,
            rpc_block.total_difficulty,
            rpc_block.pow,
            rpc_block.uncles,
        )
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct RskLog {
    info: LogInfo,
    event: LogEvent,
}

impl RskLog {
    pub fn new(info: LogInfo, event: LogEvent) -> Self {
        Self { info, event }
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

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct LogInfo {
    address: Address,
    block_hash: BlockHash,
    block_number: BlockNumber,
    tx_hash: String,
    log_index: u64,
    removed: bool,
}

impl LogInfo {
    pub fn new(
        address: Address,
        block_hash: BlockHash,
        block_number: BlockNumber,
        tx_hash: String,
        log_index: u64,
        removed: bool,
    ) -> Self {
        LogInfo {
            address,
            block_hash,
            block_number,
            tx_hash,
            log_index,
            removed,
        }
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn block_hash(&self) -> BlockHash {
        self.block_hash
    }

    pub fn block_number(&self) -> BlockNumber {
        self.block_number
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

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
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
    pub address: Address,
    pub name: String,
    pub abi: Option<JsonAbi>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RskRpcBlock {
    #[serde(deserialize_with = "parse_hex_to_block_number")]
    number: BlockNumber,
    #[serde(deserialize_with = "parse_hex_to_block_hash")]
    hash: BlockHash,
    #[serde(rename = "parentHash", deserialize_with = "parse_hex_to_block_hash")]
    parent_hash: BlockHash,
    #[serde(deserialize_with = "parse_hex_to_block_timestamp")]
    timestamp: BlockTimestamp,
    #[serde(deserialize_with = "parse_rsk_difficulty")]
    difficulty: BlockDifficulty,
    #[serde(deserialize_with = "parse_rsk_difficulty", rename = "totalDifficulty")]
    total_difficulty: BlockDifficulty,
    #[serde(
        rename = "bitcoinMergedMiningHeader",
        deserialize_with = "parse_bitcoin_header_to_pow"
    )]
    pow: BlockPow,
    #[serde(deserialize_with = "parse_uncles")]
    uncles: Vec<BlockHash>,
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

    BlockHash::try_from(hex.as_str()).map_err(de::Error::custom)
}

fn parse_rsk_difficulty<'de, D>(deserializer: D) -> Result<BlockDifficulty, D::Error>
where
    D: Deserializer<'de>,
{
    let difficulty_hex: String = Deserialize::deserialize(deserializer)?;
    let difficulty_dec = U256::from_str_radix(&difficulty_hex, 16).map_err(de::Error::custom)?;

    Ok(BlockDifficulty::from(difficulty_dec))
}

fn parse_bitcoin_header_to_pow<'de, D>(deserializer: D) -> Result<BlockPow, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;

    BlockPow::try_from(hex.as_str()).map_err(de::Error::custom)
}

fn parse_uncles<'de, D>(deserializer: D) -> Result<Vec<BlockHash>, D::Error>
where
    D: Deserializer<'de>,
{
    let hex_strings: Vec<Value> = Deserialize::deserialize(deserializer)?;

    hex_strings
        .into_iter()
        .map(|v| parse_hex_to_block_hash(v).map_err(de::Error::custom))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::test_utils::rsk_utils::{DEFAULT_BITCOIN_MERGED_MINING_HEADER, DEFAULT_BLOCK_HASH};
    use crate::types::{BlockHash, BlockPow};

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

        if let Err(_) = block_hash {
            // The error was expected due to invalid hex input
        } else {
            panic!("Expected Error, but got: {:?}", block_hash);
        }
    }

    #[test]
    fn test_missing_prefix_when_hash_without_prefix_is_provided_should_return_ok() {
        let valid_hash_without_prefix = &DEFAULT_BLOCK_HASH[2..];
        let block_hash = BlockHash::try_from(valid_hash_without_prefix);

        assert!(block_hash.is_ok());
    }

    #[test]
    fn test_valid_block_pow_when_valid_bitcoin_merged_mining_header_is_provided_should_return_ok() {
        let pow = BlockPow::try_from(DEFAULT_BITCOIN_MERGED_MINING_HEADER);

        assert!(pow.is_ok());
    }

    #[test]
    fn test_invalid_block_pow_when_invalid_merged_mining_header_is_provided_should_return_error() {
        let invalid_header = "0xinvalidheader";
        let pow = BlockPow::try_from(invalid_header);

        assert!(pow.is_err());

        if let Err(_) = pow {
            // The error was expected due to invalid hex input
        } else {
            panic!("Expected Error, but got: {:?}", pow);
        }
    }

    #[test]
    fn test_missing_prefix_when_bitcoin_merged_mining_header_without_prefix_is_provided_should_return_ok(
    ) {
        let valid_hash_without_prefix = &DEFAULT_BITCOIN_MERGED_MINING_HEADER[2..];
        let pow = BlockPow::try_from(valid_hash_without_prefix);

        assert!(pow.is_ok());
    }
}
