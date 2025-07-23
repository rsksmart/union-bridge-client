use alloy_json_abi::JsonAbi;
use alloy_primitives::FixedBytes;
use anyhow::Result;
use bitcoin::{blockdata::block::Header, consensus::encode::deserialize as btc_deserialize};
use hex::FromHexError;
use log::error;
use musig2::PubNonce;
use primitive_types::{H160, H256, U256};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;
use std::{
    cmp::Ordering,
    fmt,
    num::ParseIntError,
    ops::{Add, Mul, Sub},
    str::FromStr,
    string::ToString,
};

/// A trait for types that can be converted into a hexadecimal string.
///
/// Implement this trait to provide a computer-friendly, lowercase hex
/// representation of the underlying value. This is useful for serializing
/// numerical values or identifiers in blockchain, networking, or low-level
/// data applications.
pub trait ToHexString {
    fn to_hex_string(&self) -> String;
}

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
#[derive(Serialize, Deserialize, Copy, Debug, Eq, PartialEq, Hash, Clone, Default)]
pub struct Hash256(H256);

impl Hash256 {
    pub fn value(self) -> H256 {
        self.0
    }
}

impl From<H256> for Hash256 {
    fn from(h256: H256) -> Self {
        Self(h256)
    }
}

impl From<H160> for Hash256 {
    fn from(h160: H160) -> Self {
        Self(H256::from(h160))
    }
}

impl TryFrom<&str> for Hash256 {
    type Error = FromHexError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let value = value.trim_start_matches("0x");
        let bytes = hex::decode(value)?;
        let h256 = H256::from_slice(&bytes);

        Ok(Self(h256))
    }
}

impl From<FixedBytes<32>> for Hash256 {
    fn from(bytes: FixedBytes<32>) -> Self {
        Hash256::from(H256::from_slice(&bytes.0))
    }
}

impl From<Hash256> for FixedBytes<32> {
    fn from(hash: Hash256) -> Self {
        FixedBytes::<32>::from_slice(hash.value().as_bytes())
    }
}

impl From<PubNonce> for Hash256 {
    fn from(nonce: PubNonce) -> Self {
        Hash256::from(H256::from_slice(nonce.serialize().as_slice()))
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

pub type BlockHash = Hash256;
pub type TxHash = Hash256;
pub type LogTopic = Hash256;

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
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Copy, Clone)]
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

impl TryFrom<&str> for BlockNumber {
    type Error = ParseIntError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let result = str_hex_to_u64(value.to_string())?;

        Ok(BlockNumber(result))
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

impl ToHexString for BlockNumber {
    fn to_hex_string(&self) -> String {
        format!("{:#x}", self.0)
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
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
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
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, PartialOrd, Copy, Clone)]
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
#[derive(Serialize, Deserialize, Copy, Debug, Eq, PartialEq, Clone)]
pub struct BlockPow(H256);

impl BlockPow {
    pub fn value(self) -> H256 {
        self.0
    }

    pub fn into_effort(self) -> U256 {
        let pow: U256 = U256::from_big_endian(self.value().as_bytes());
        // compute the effort by inverting the pow
        // U256::MAX, the "difficulty 1" target, represents the easiest possible target
        U256::MAX.checked_div(pow).unwrap_or_else(|| {
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("0 division on pow_to_effort");
            U256::zero()
        })
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
#[derive(
    Serialize, Deserialize, Copy, Debug, Ord, PartialOrd, PartialEq, Eq, Clone, Hash, Default,
)]
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

impl From<alloy_primitives::Address> for Address {
    fn from(addr: alloy_primitives::Address) -> Self {
        Self(H160::from_slice(addr.as_slice()))
    }
}

impl From<Address> for alloy_primitives::Address {
    fn from(addr: Address) -> Self {
        Self(*alloy_primitives::Address::from_slice(
            addr.0.as_fixed_bytes(),
        ))
    }
}

impl TryFrom<&str> for Address {
    type Error = FromHexError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let value = value.trim_start_matches("0x");
        let bytes = hex::decode(value)?;
        let h160 = H160::from_slice(&bytes);

        Ok(Self(h160))
    }
}

impl ToHexString for Address {
    fn to_hex_string(&self) -> String {
        format!("{:#x}", self.0)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex_string())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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

impl std::hash::Hash for RskBlock {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for RskBlock {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl Eq for RskBlock {
    // derived from PartialEq
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

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
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

impl From<RskRpcLog> for RskLog {
    fn from(rpc_log: RskRpcLog) -> Self {
        Self::new(
            LogInfo::new(
                rpc_log.address,
                rpc_log.block_hash,
                rpc_log.block_number,
                rpc_log.tx_hash,
                rpc_log.log_index,
                // assumption is made where the log is canonical if coming from request (not subscription)
                false,
            ),
            LogEvent::new(rpc_log.data, rpc_log.topics),
        )
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

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub struct LogInfo {
    address: Address,
    block_hash: BlockHash,
    block_number: BlockNumber,
    tx_hash: TxHash,
    log_index: u64,
    removed: bool,
}

impl LogInfo {
    pub fn new(
        address: Address,
        block_hash: BlockHash,
        block_number: BlockNumber,
        tx_hash: TxHash,
        log_index: u64,
        removed: bool,
    ) -> Self {
        Self {
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

    pub fn tx_hash(&self) -> TxHash {
        self.tx_hash
    }

    pub fn log_index(&self) -> u64 {
        self.log_index
    }

    pub fn removed(&self) -> bool {
        self.removed
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataBytes(pub Vec<u8>);

impl DataBytes {
    pub fn new(data: Vec<u8>) -> Self {
        Self(data)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_hex_str(s: &str) -> Result<Self, hex::FromHexError> {
        let clean = s.trim_start_matches("0x");
        hex::decode(clean).map(Self)
    }

    pub fn to_hex_string(&self) -> String {
        format!("0x{}", hex::encode(&self.0))
    }
}

impl fmt::Display for DataBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex_string())
    }
}

impl AsRef<[u8]> for DataBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<String> for DataBytes {
    type Error = hex::FromHexError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        DataBytes::from_hex_str(&value)
    }
}

impl From<DataBytes> for String {
    fn from(data: DataBytes) -> Self {
        data.to_hex_string()
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct LogEvent {
    data: DataBytes,
    topics: Vec<LogTopic>,
}

impl LogEvent {
    pub fn new(data: DataBytes, topics: Vec<LogTopic>) -> Self {
        Self { data, topics }
    }

    pub fn data(&self) -> &DataBytes {
        &self.data
    }

    pub fn topics(&self) -> &Vec<LogTopic> {
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

    #[serde(deserialize_with = "parse_hex_to_hash256")]
    hash: BlockHash,

    #[serde(rename = "parentHash", deserialize_with = "parse_hex_to_hash256")]
    parent_hash: BlockHash,

    #[serde(deserialize_with = "parse_hex_to_block_timestamp")]
    timestamp: BlockTimestamp,

    #[serde(deserialize_with = "parse_rsk_difficulty")]
    difficulty: BlockDifficulty,

    #[serde(deserialize_with = "parse_rsk_difficulty", rename = "totalDifficulty")]
    total_difficulty: BlockDifficulty,

    #[cfg_attr(
        not(feature = "anvil"),
        serde(
            rename = "bitcoinMergedMiningHeader",
            deserialize_with = "parse_bitcoin_header_to_pow"
        )
    )]
    #[cfg_attr(
        feature = "anvil",
        serde(
            default = "default_pow_header",
            rename = "bitcoinMergedMiningHeader",
            deserialize_with = "parse_bitcoin_header_to_pow"
        )
    )]
    pow: BlockPow,

    #[serde(deserialize_with = "parse_hash256_vec")]
    uncles: Vec<BlockHash>,
}

#[cfg(feature = "anvil")]
fn default_pow_header() -> BlockPow {
    use crate::anvil_mocks::get_anvil_block_pow;
    get_anvil_block_pow()
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RskRpcLog {
    #[serde(deserialize_with = "parse_hex_to_address")]
    address: Address,

    #[serde(rename = "blockHash", deserialize_with = "parse_hex_to_hash256")]
    block_hash: BlockHash,

    #[serde(rename = "blockNumber", deserialize_with = "parse_hex_to_block_number")]
    block_number: BlockNumber,

    #[serde(rename = "transactionHash", deserialize_with = "parse_hex_to_hash256")]
    tx_hash: TxHash,

    #[serde(rename = "logIndex", deserialize_with = "parse_hex_to_u64")]
    log_index: u64,

    #[serde(deserialize_with = "parse_hex_to_data_bytes")]
    data: DataBytes,

    #[serde(deserialize_with = "parse_hash256_vec")]
    topics: Vec<LogTopic>,
    // no "removed" field if coming from request (not subscription)
}

fn parse_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;
    str_hex_to_u64(hex).map_err(de::Error::custom)
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

fn parse_hex_to_hash256<'de, D>(deserializer: D) -> Result<Hash256, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;

    Hash256::try_from(hex.as_str()).map_err(de::Error::custom)
}

fn parse_hex_to_address<'de, D>(deserializer: D) -> Result<Address, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;

    Address::try_from(hex.as_str()).map_err(de::Error::custom)
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

fn parse_hash256_vec<'de, D>(deserializer: D) -> Result<Vec<Hash256>, D::Error>
where
    D: Deserializer<'de>,
{
    let hex_strings: Vec<Value> = Deserialize::deserialize(deserializer)?;

    hex_strings
        .into_iter()
        .map(|v| parse_hex_to_hash256(v).map_err(de::Error::custom))
        .collect()
}

fn str_hex_to_u64(hex: String) -> Result<u64, ParseIntError> {
    u64::from_str_radix(hex.trim_start_matches("0x"), 16)
}

fn parse_hex_to_data_bytes<'de, D>(deserializer: D) -> Result<DataBytes, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;

    DataBytes::from_hex_str(hex.as_str()).map_err(de::Error::custom)
}

#[cfg(test)]
mod tests {
    use crate::test_utils::rsk_utils::{DEFAULT_BITCOIN_MERGED_MINING_HEADER, DEFAULT_BLOCK_HASH};
    use crate::types::{BlockHash, BlockPow, DataBytes};

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
    fn test_missing_prefix_when_bitcoin_merged_mining_header_without_prefix_is_provided_should_return_ok()
     {
        let valid_hash_without_prefix = &DEFAULT_BITCOIN_MERGED_MINING_HEADER[2..];
        let pow = BlockPow::try_from(valid_hash_without_prefix);

        assert!(pow.is_ok());
    }

    #[test]
    fn test_from_hex_str_with_0x_prefix() {
        let hex = "0xdeadbeef";
        let bytes = DataBytes::from_hex_str(hex).expect("Failed to parse hex");
        assert_eq!(bytes.0, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn test_from_hex_str_without_prefix() {
        let hex = "deadbeef";
        let bytes = DataBytes::from_hex_str(hex).expect("Failed to parse hex");
        assert_eq!(bytes.0, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn test_to_hex_string() {
        let data = DataBytes(vec![0xca, 0xfe, 0xba, 0xbe]);
        assert_eq!(data.to_hex_string(), "0xcafebabe");
    }

    #[test]
    fn test_display_impl() {
        let data = DataBytes(vec![0xca, 0xfe, 0xba, 0xbe]);
        assert_eq!(format!("{}", data), "0xcafebabe");
    }

    #[test]
    fn test_try_from_string() {
        let hex_string = String::from("0x1234abcd");
        let bytes = DataBytes::try_from(hex_string).expect("Conversion failed");
        assert_eq!(bytes.0, vec![0x12, 0x34, 0xab, 0xcd]);
    }

    #[test]
    fn test_from_data_bytes_to_string() {
        let data = DataBytes(vec![0x01, 0x02, 0x03]);
        let s: String = data.into();
        assert_eq!(s, "0x010203");
    }

    #[test]
    fn test_invalid_hex_should_fail() {
        let invalid = "0xxyz123";
        let result = DataBytes::from_hex_str(invalid);
        assert!(result.is_err());
    }
}

#[derive(Eq, PartialEq, Serialize, Deserialize, Debug, Clone)]
pub struct RskBlockAndUncles {
    block: RskBlock,
    uncles: Vec<RskBlock>,
}

impl RskBlockAndUncles {
    pub fn new(block: RskBlock, uncles: Vec<RskBlock>) -> Self {
        Self { block, uncles }
    }

    pub fn new_no_uncles(block: RskBlock) -> Self {
        Self {
            block,
            uncles: vec![],
        }
    }

    pub fn hash(&self) -> BlockHash {
        self.block.hash()
    }

    pub fn parent(&self) -> BlockHash {
        self.block.parent_hash()
    }

    pub fn number(&self) -> BlockNumber {
        self.block.number()
    }

    pub fn block(&self) -> &RskBlock {
        &self.block
    }

    pub fn uncles(&self) -> &[RskBlock] {
        &self.uncles
    }
}
