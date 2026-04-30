use std::cmp::Ordering;
use std::fmt;
use std::num::ParseIntError;
use std::ops::{Add, Mul, Sub};
use std::str::FromStr;
use std::string::ToString;

use alloy_primitives::FixedBytes;
use anyhow::{Result, bail};
use bitcoin::Txid;
use bitcoin::blockdata::block::Header;
use bitcoin::consensus::encode::deserialize as btc_deserialize;
use bitcoin::hashes::Hash;
use hex::FromHexError;
use log::error;
use musig2::PubNonce;
use primitive_types::{H160, H256, U256};
use serde::{Deserialize, Deserializer, Serialize, de};

/// A trait for types that can be converted into a hexadecimal string.
///
/// Implement this trait to provide a computer-friendly, lowercase hex
/// representation of the underlying value. This is useful for serializing
/// numerical values or identifiers in blockchain, networking, or low-level
/// data applications.
pub trait ToHexString {
    fn to_hex_string(&self) -> String;
}

/// Represents a rootstock block hash.
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
    #[must_use]
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
    #[must_use]
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
        let result = str_hex_to_u64(value)?;

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
    #[must_use]
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
    #[must_use]
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
    #[must_use]
    pub fn value(self) -> H256 {
        self.0
    }

    #[must_use]
    pub fn into_effort(self) -> U256 {
        let pow: U256 = U256::from_big_endian(self.value().as_bytes());
        // compute the effort by inverting the pow
        // U256::MAX, the "difficulty 1" target, represents the easiest possible target
        U256::MAX.checked_div(pow).unwrap_or_else(|| {
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
        let header_hash = btc_deserialize::<Header>(&header_bytes)?.block_hash().to_string();
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
    #[must_use]
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
        Self(*alloy_primitives::Address::from_slice(addr.0.as_fixed_bytes()))
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
    // Keep serde defaults for new header fields so non-RSK/local providers and old serialized
    // snapshots do not fail to deserialize while we still run mixed environments.
    #[serde(default)]
    uncles_hash: BlockHash,
    #[serde(default)]
    miner: Address,
    #[serde(default)]
    state_root: BlockHash,
    #[serde(default)]
    transactions_root: BlockHash,
    #[serde(default)]
    receipts_root: BlockHash,
    #[serde(default = "default_logs_bloom_bytes")]
    logs_bloom: DataBytes,
    #[serde(default = "default_gas_limit_bytes")]
    gas_limit: DataBytes,
    #[serde(default)]
    gas_used: u64,
    #[serde(default = "default_empty_data_bytes")]
    extra_data: DataBytes,
    #[serde(default = "default_zero_u256")]
    paid_fees: U256,
    #[serde(default = "default_minimum_gas_price")]
    minimum_gas_price: Option<U256>,
    #[serde(default = "default_merged_mining_header_bytes")]
    bitcoin_merged_mining_header: DataBytes,
    #[serde(default)]
    rsk_pte_edges: Option<Vec<u16>>,
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
    #[allow(clippy::too_many_arguments)]
    #[must_use]
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
            uncles_hash: BlockHash::from(H256::zero()),
            miner: Address::from(H160::zero()),
            state_root: BlockHash::from(H256::zero()),
            transactions_root: BlockHash::from(H256::zero()),
            receipts_root: BlockHash::from(H256::zero()),
            logs_bloom: DataBytes(vec![0u8; 256]),
            gas_limit: DataBytes(vec![0u8]),
            gas_used: 0,
            extra_data: DataBytes::new(Vec::new()),
            paid_fees: U256::zero(),
            minimum_gas_price: Some(U256::zero()),
            bitcoin_merged_mining_header: DataBytes(vec![0u8; 80]),
            rsk_pte_edges: None,
        }
    }

    #[must_use]
    pub fn number(&self) -> BlockNumber {
        self.number
    }

    #[must_use]
    pub fn hash(&self) -> BlockHash {
        self.hash
    }

    #[must_use]
    pub fn parent_hash(&self) -> BlockHash {
        self.parent_hash
    }

    #[must_use]
    pub fn timestamp(&self) -> BlockTimestamp {
        self.timestamp
    }

    #[must_use]
    pub fn difficulty(&self) -> BlockDifficulty {
        self.difficulty
    }

    #[must_use]
    pub fn total_difficulty(&self) -> BlockDifficulty {
        self.total_difficulty
    }

    #[must_use]
    pub fn pow(&self) -> BlockPow {
        self.pow
    }

    #[must_use]
    pub fn uncles(&self) -> Vec<BlockHash> {
        self.uncles.clone()
    }

    #[must_use]
    pub fn uncles_hash(&self) -> BlockHash {
        self.uncles_hash
    }

    #[must_use]
    pub fn miner(&self) -> Address {
        self.miner
    }

    #[must_use]
    pub fn state_root(&self) -> BlockHash {
        self.state_root
    }

    #[must_use]
    pub fn transactions_root(&self) -> BlockHash {
        self.transactions_root
    }

    #[must_use]
    pub fn receipts_root(&self) -> BlockHash {
        self.receipts_root
    }

    #[must_use]
    pub fn logs_bloom(&self) -> &DataBytes {
        &self.logs_bloom
    }

    #[must_use]
    pub fn gas_limit(&self) -> &DataBytes {
        &self.gas_limit
    }

    #[must_use]
    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    #[must_use]
    pub fn extra_data(&self) -> &DataBytes {
        &self.extra_data
    }

    #[must_use]
    pub fn paid_fees(&self) -> U256 {
        self.paid_fees
    }

    #[must_use]
    pub fn minimum_gas_price(&self) -> Option<U256> {
        self.minimum_gas_price
    }

    #[must_use]
    pub fn bitcoin_merged_mining_header(&self) -> &DataBytes {
        &self.bitcoin_merged_mining_header
    }

    #[must_use]
    pub fn rsk_pte_edges(&self) -> Option<&[u16]> {
        self.rsk_pte_edges.as_deref()
    }
}

impl From<RskRpcBlock> for RskBlock {
    fn from(rpc_block: RskRpcBlock) -> Self {
        let RskRpcBlock {
            number,
            hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            uncles_hash,
            miner,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            gas_limit,
            gas_used,
            extra_data,
            paid_fees,
            minimum_gas_price,
            bitcoin_merged_mining,
            rsk_pte_edges,
            uncles,
        } = rpc_block;

        RskBlock {
            number,
            hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            pow: bitcoin_merged_mining.pow,
            uncles,
            uncles_hash,
            miner,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            gas_limit,
            gas_used,
            extra_data,
            paid_fees,
            minimum_gas_price,
            bitcoin_merged_mining_header: bitcoin_merged_mining.header,
            rsk_pte_edges,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct RskLog {
    info: LogInfo,
    event: LogEvent,
}

impl RskLog {
    #[must_use]
    pub fn new(info: LogInfo, event: LogEvent) -> Self {
        Self { info, event }
    }

    #[must_use]
    pub fn info(&self) -> &LogInfo {
        &self.info
    }

    #[must_use]
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
    #[must_use]
    pub fn new(
        address: Address,
        block_hash: BlockHash,
        block_number: BlockNumber,
        tx_hash: TxHash,
        log_index: u64,
        removed: bool,
    ) -> Self {
        Self { address, block_hash, block_number, tx_hash, log_index, removed }
    }

    #[must_use]
    pub fn address(&self) -> Address {
        self.address
    }

    #[must_use]
    pub fn block_hash(&self) -> BlockHash {
        self.block_hash
    }

    #[must_use]
    pub fn block_number(&self) -> BlockNumber {
        self.block_number
    }

    #[must_use]
    pub fn tx_hash(&self) -> TxHash {
        self.tx_hash
    }

    #[must_use]
    pub fn log_index(&self) -> u64 {
        self.log_index
    }

    #[must_use]
    pub fn removed(&self) -> bool {
        self.removed
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataBytes(pub Vec<u8>);

impl DataBytes {
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self(data)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// # Errors
    ///
    /// Returns an error if the hex string cannot be parsed.
    pub fn from_hex_str(s: &str) -> Result<Self, hex::FromHexError> {
        let clean = s.trim_start_matches("0x");
        hex::decode(clean).map(Self)
    }

    #[must_use]
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
    #[must_use]
    pub fn new(data: DataBytes, topics: Vec<LogTopic>) -> Self {
        Self { data, topics }
    }

    #[must_use]
    pub fn data(&self) -> &DataBytes {
        &self.data
    }

    #[must_use]
    pub fn topics(&self) -> &Vec<LogTopic> {
        &self.topics
    }
}

#[derive(Debug, Clone)]
pub struct ContractInfo {
    pub address: Address,
    pub name: String,
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

    #[serde(rename = "sha3Uncles", deserialize_with = "parse_hex_to_hash256")]
    uncles_hash: BlockHash,

    #[serde(rename = "miner", deserialize_with = "parse_hex_to_address")]
    miner: Address,

    #[serde(rename = "stateRoot", deserialize_with = "parse_hex_to_hash256")]
    state_root: BlockHash,

    #[serde(rename = "transactionsRoot", deserialize_with = "parse_hex_to_hash256")]
    transactions_root: BlockHash,

    #[serde(rename = "receiptsRoot", deserialize_with = "parse_hex_to_hash256")]
    receipts_root: BlockHash,

    #[serde(rename = "logsBloom", deserialize_with = "parse_hex_to_data_bytes")]
    logs_bloom: DataBytes,

    #[serde(rename = "gasLimit", deserialize_with = "parse_hex_quantity_to_data_bytes")]
    gas_limit: DataBytes,

    #[serde(rename = "gasUsed", deserialize_with = "parse_hex_to_u64")]
    gas_used: u64,

    #[serde(rename = "extraData", deserialize_with = "parse_hex_to_data_bytes")]
    extra_data: DataBytes,

    #[serde(
        default = "default_zero_u256",
        rename = "paidFees",
        deserialize_with = "parse_hex_to_u256"
    )]
    paid_fees: U256,

    #[serde(
        default = "default_minimum_gas_price",
        rename = "minimumGasPrice",
        deserialize_with = "parse_optional_hex_to_u256"
    )]
    minimum_gas_price: Option<U256>,

    #[cfg_attr(
        not(feature = "anvil"),
        serde(
            rename = "bitcoinMergedMiningHeader",
            deserialize_with = "parse_bitcoin_merged_mining_data"
        )
    )]
    #[cfg_attr(
        feature = "anvil",
        serde(
            default = "default_bitcoin_merged_mining_data",
            rename = "bitcoinMergedMiningHeader",
            deserialize_with = "parse_bitcoin_merged_mining_data"
        )
    )]
    bitcoin_merged_mining: RskRpcBitcoinMergedMining,

    #[serde(deserialize_with = "parse_hash256_vec")]
    uncles: Vec<BlockHash>,

    #[serde(default, rename = "rskPteEdges", deserialize_with = "parse_optional_u16_vec")]
    rsk_pte_edges: Option<Vec<u16>>,
}

fn default_zero_u256() -> U256 {
    U256::zero()
}

#[allow(clippy::unnecessary_wraps)] // serde default for Option<U256> requires an Option-returning fn
fn default_minimum_gas_price() -> Option<U256> {
    Some(U256::zero())
}

fn default_logs_bloom_bytes() -> DataBytes {
    DataBytes(vec![0u8; 256])
}

fn default_gas_limit_bytes() -> DataBytes {
    DataBytes(vec![0u8])
}

fn default_empty_data_bytes() -> DataBytes {
    DataBytes::new(Vec::new())
}

fn default_merged_mining_header_bytes() -> DataBytes {
    DataBytes(vec![0u8; 80])
}

#[cfg(feature = "anvil")]
fn default_bitcoin_merged_mining_data() -> RskRpcBitcoinMergedMining {
    use crate::anvil_mocks::get_anvil_block_pow;
    RskRpcBitcoinMergedMining {
        header: default_merged_mining_header_bytes(),
        pow: get_anvil_block_pow(),
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct RskRpcBitcoinMergedMining {
    header: DataBytes,
    pow: BlockPow,
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

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Hash, Clone)]
pub struct CommitteeId(u128);

impl std::fmt::Display for CommitteeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u128> for CommitteeId {
    fn from(value: u128) -> Self {
        CommitteeId(value)
    }
}

impl std::ops::Deref for CommitteeId {
    type Target = u128;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// contracts binding generate a U256 for the committeeId in some events (when indexed, for the
// required hashing I think), but under the hood in the contracts it is an u128
impl TryFrom<alloy_primitives::Uint<256, 4>> for CommitteeId {
    type Error = anyhow::Error;

    fn try_from(value: alloy_primitives::Uint<256, 4>) -> Result<Self> {
        match value.try_into() {
            Ok(num) => Ok(CommitteeId(num)),
            Err(e) => bail!("Failed to convert Uint<256,4> {value:?} to CommitteeId: {e}"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct StreamId(u64);

// contracts store streamId as u64, but only accept u8 on StreamDenomination struct
impl StreamId {
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// # Errors
    ///
    /// Returns an error if the value cannot be converted to u8.
    pub fn as_u8(&self) -> Result<u8> {
        let val = *self.clone();
        let result = u8::try_from(val);
        match result {
            Ok(num) => Ok(num),
            Err(e) => bail!("Failed to convert StreamId {val} to u8: {e}"),
        }
    }
}

impl From<u64> for StreamId {
    fn from(value: u64) -> Self {
        StreamId(value)
    }
}

impl std::ops::Deref for StreamId {
    type Target = u64;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn parse_hex_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;
    str_hex_to_u64(&hex).map_err(de::Error::custom)
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
    let difficulty_dec = U256::from_str_radix(difficulty_hex.trim_start_matches("0x"), 16)
        .map_err(de::Error::custom)?;

    Ok(BlockDifficulty::from(difficulty_dec))
}

fn parse_hex_to_u256<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;
    U256::from_str_radix(hex.trim_start_matches("0x"), 16).map_err(de::Error::custom)
}

fn parse_optional_hex_to_u256<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: Option<String> = Option::deserialize(deserializer)?;
    hex.map(|v| U256::from_str_radix(v.trim_start_matches("0x"), 16).map_err(de::Error::custom))
        .transpose()
}

fn parse_bitcoin_merged_mining_data<'de, D>(
    deserializer: D,
) -> Result<RskRpcBitcoinMergedMining, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;
    let header = DataBytes::from_hex_str(hex.as_str()).map_err(de::Error::custom)?;
    let pow = BlockPow::try_from(hex.as_str()).map_err(de::Error::custom)?;

    Ok(RskRpcBitcoinMergedMining { header, pow })
}

fn parse_hash256_vec<'de, D>(deserializer: D) -> Result<Vec<Hash256>, D::Error>
where
    D: Deserializer<'de>,
{
    let hex_strings: Vec<String> = Deserialize::deserialize(deserializer)?;

    hex_strings
        .into_iter()
        .map(|hex| Hash256::try_from(hex.as_str()).map_err(de::Error::custom))
        .collect()
}

fn parse_optional_u16_vec<'de, D>(deserializer: D) -> Result<Option<Vec<u16>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<u16>>::deserialize(deserializer)
}

fn str_hex_to_u64(hex: &str) -> Result<u64, ParseIntError> {
    u64::from_str_radix(hex.trim_start_matches("0x"), 16)
}

fn parse_hex_to_data_bytes<'de, D>(deserializer: D) -> Result<DataBytes, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;

    DataBytes::from_hex_str(hex.as_str()).map_err(de::Error::custom)
}

fn parse_hex_quantity_to_data_bytes<'de, D>(deserializer: D) -> Result<DataBytes, D::Error>
where
    D: Deserializer<'de>,
{
    let hex: String = Deserialize::deserialize(deserializer)?;
    let clean = hex.trim_start_matches("0x");
    let normalized = if clean.is_empty() { "0" } else { clean };

    let padded =
        if normalized.len() % 2 == 0 { normalized.to_string() } else { format!("0{normalized}") };

    hex::decode(padded).map(DataBytes).map_err(de::Error::custom)
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

        if block_hash.is_err() {
            // The error was expected due to invalid hex input
        } else {
            panic!("Expected Error, but got: {block_hash:?}");
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

        if pow.is_err() {
            // The error was expected due to invalid hex input
        } else {
            panic!("Expected Error, but got: {pow:?}");
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
        assert_eq!(format!("{data}"), "0xcafebabe");
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
    #[must_use]
    pub fn new(block: RskBlock, uncles: Vec<RskBlock>) -> Self {
        Self { block, uncles }
    }

    #[must_use]
    pub fn new_no_uncles(block: RskBlock) -> Self {
        Self { block, uncles: vec![] }
    }

    #[must_use]
    pub fn hash(&self) -> BlockHash {
        self.block.hash()
    }

    #[must_use]
    pub fn parent(&self) -> BlockHash {
        self.block.parent_hash()
    }

    #[must_use]
    pub fn number(&self) -> BlockNumber {
        self.block.number()
    }

    #[must_use]
    pub fn block(&self) -> &RskBlock {
        &self.block
    }

    #[must_use]
    pub fn uncles(&self) -> &[RskBlock] {
        &self.uncles
    }
}

/// bitcoin / `bitcoin_hashes` crates reverse the byte order of Txid when calling `from_byte_array`, `from_slice`, etc.
/// this utility struct provides conversion methods to handle that, so no other occurrence of those methods should be used outside this struct
pub struct TxIdParser;
impl TxIdParser {
    #[must_use]
    pub fn fb_32_to_txid(tx_id: FixedBytes<32>) -> Txid {
        let mut bytes: [u8; 32] = tx_id.into();
        bytes.reverse();
        Txid::from_byte_array(bytes)
    }

    #[must_use]
    pub fn txid_to_fb_32(txid: Txid) -> FixedBytes<32> {
        let mut bytes = txid.to_byte_array();
        bytes.reverse();
        FixedBytes::<32>::from_slice(&bytes)
    }
}
