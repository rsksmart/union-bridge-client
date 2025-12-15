#![allow(clippy::missing_errors_doc)]

use primitive_types::H256;
use primitive_types::U256;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use sha3::Digest;
use sha3::Keccak256;
use std::fmt;

use crate::rlp::encode_coin_value;
use crate::rlp::encode_signed_coin_value;

#[derive(Serialize, Deserialize, Clone)]
pub struct RskBlockHeader {
    #[serde(rename = "number", deserialize_with = "deserialize_hex_u64")]
    pub number: u64, // Block height (genesis = 0)
    #[serde(rename = "hash", deserialize_with = "deserialize_hex_h256")]
    pub hash: H256, // Keccak-256 of the encoded header
    #[serde(rename = "parentHash", deserialize_with = "deserialize_hex_h256")]
    pub parent: H256, // Keccak-256 hash of the parent block
    #[serde(rename = "difficulty", deserialize_with = "deserialize_hex_u256")]
    pub difficulty: U256, // Target difficulty for this block
    #[serde(rename = "timestamp", deserialize_with = "deserialize_hex_u64")]
    pub timestamp: u64, // Unix time (seconds) when the block was created
    #[serde(rename = "sha3Uncles", deserialize_with = "deserialize_hex_h256")]
    pub uncles_hash: H256, // SHA3-256 hash of the uncles list
    #[serde(rename = "miner", deserialize_with = "deserialize_hex_bytes_20")]
    pub coinbase: [u8; 20], // 160-bit address (RskAddress) - miner's address
    #[serde(rename = "stateRoot", deserialize_with = "deserialize_hex_h256")]
    pub state_root: H256, // SHA3-256 hash of the root node of the state trie
    #[serde(rename = "transactionsRoot", deserialize_with = "deserialize_hex_h256")]
    pub tx_trie_root: H256, // SHA3-256 hash of the root node of the transaction trie
    #[serde(rename = "receiptsRoot", deserialize_with = "deserialize_hex_h256")]
    pub receipt_trie_root: H256, // SHA3-256 hash of the root node of the receipt trie
    #[serde(rename = "logsBloom", deserialize_with = "deserialize_hex_bytes")]
    pub logs_bloom: Vec<u8>, // Bloom filter for logs (256 bytes) or extension_data if RSKIP-351
    #[serde(rename = "gasLimit", deserialize_with = "deserialize_hex_bytes")]
    pub gas_limit: Vec<u8>, // Current limit of gas expenditure per block
    #[serde(rename = "gasUsed", deserialize_with = "deserialize_hex_u64")]
    pub gas_used: u64, // Total gas used in transactions in this block
    #[serde(rename = "extraData", deserialize_with = "deserialize_hex_bytes")]
    pub extra_data: Vec<u8>, // Arbitrary byte array (max 32 bytes, except genesis)
    #[serde(rename = "paidFees", deserialize_with = "deserialize_hex_u256")]
    pub paid_fees: U256, // Total paid fees in transactions (Coin, RLP encoded)
    #[serde(
        rename = "minimumGasPrice",
        deserialize_with = "deserialize_hex_u256_option"
    )]
    pub minimum_gas_price: Option<U256>, // Minimum gas price for a tx to be included
    #[serde(rename = "uncles", deserialize_with = "deserialize_vec_hex_h256")]
    pub uncles: Vec<H256>, // Hashes of uncle blocks
    #[serde(
        rename = "bitcoinMergedMiningHeader",
        deserialize_with = "deserialize_hex_bytes"
    )]
    pub bitcoin_merged_mining_header: Vec<u8>, // 80-byte Bitcoin block header for merged mining
    // the follwoing fields are goonna be included in the next hardfork (reed)
    #[serde(skip)]
    _umm_root: [u8; 20], // UMM root (only if block is UMM, must be exactly 20 bytes)
    #[serde(skip)]
    _version: u8, // Header version
    #[serde(skip)]
    _tx_execution_sublists_edges: Option<Vec<u16>>, // Edges of transaction execution sublists
}

impl Default for RskBlockHeader {
    fn default() -> Self {
        Self {
            number: 0,
            hash: H256::zero(),
            parent: H256::zero(),
            difficulty: U256::zero(),
            timestamp: 0,
            uncles_hash: H256::zero(),
            coinbase: [0u8; 20],
            state_root: H256::zero(),
            tx_trie_root: H256::zero(),
            receipt_trie_root: H256::zero(),
            logs_bloom: vec![0u8; 256],
            gas_limit: vec![0u8],
            gas_used: 0,
            extra_data: Vec::new(),
            paid_fees: U256::zero(),
            minimum_gas_price: Some(U256::zero()),
            uncles: Vec::new(),
            bitcoin_merged_mining_header: vec![0u8; 80],
            _umm_root: [0u8; 20],
            _version: 0,
            _tx_execution_sublists_edges: None,
        }
    }
}

impl RskBlockHeader {
    #[must_use]
    pub fn new_with(number: u64, difficulty: U256, parent: Option<H256>, timestamp: u64) -> Self {
        RskBlockHeader {
            number,
            difficulty,
            parent: parent.unwrap_or_default(),
            timestamp,
            ..Default::default()
        }
    }

    pub fn calculate_block_hash(&self) -> Result<H256, &'static str> {
        let rlp_encoded: Vec<u8> = self.encode_rlp()?;
        let mut hasher = Keccak256::new();
        hasher.update(&rlp_encoded);
        let block_hash = H256::from_slice(&hasher.finalize());
        Ok(block_hash)
    }

    pub fn encode_rlp(&self) -> Result<Vec<u8>, &'static str> {
        let Some(minimum_gas_price) = self.minimum_gas_price else {
            return Err("minimum_gas_price is None");
        };
        let encoded_fields: Vec<Vec<u8>> = vec![
            alloy_rlp::encode(self.parent.as_bytes()),
            alloy_rlp::encode(self.uncles_hash.as_bytes()),
            alloy_rlp::encode(self.coinbase.as_slice()),
            alloy_rlp::encode(self.state_root.as_bytes()),
            alloy_rlp::encode(self.tx_trie_root.as_bytes()),
            alloy_rlp::encode(self.receipt_trie_root.as_bytes()),
            alloy_rlp::encode(self.logs_bloom.as_slice()),
            encode_coin_value(&self.difficulty),
            alloy_rlp::encode(self.number),
            alloy_rlp::encode(self.gas_limit.as_slice()),
            alloy_rlp::encode(self.gas_used),
            alloy_rlp::encode(self.timestamp),
            alloy_rlp::encode(self.extra_data.as_slice()),
            encode_coin_value(&self.paid_fees),
            encode_signed_coin_value(&minimum_gas_price),
            alloy_rlp::encode(self.uncles.len()), // uncle_count
            alloy_rlp::encode::<&[u8]>(&self.umm_root()),
            alloy_rlp::encode(self.bitcoin_merged_mining_header.as_slice()),
        ];
        let out = encode_list(encoded_fields);
        Ok(out)
    }
    #[must_use]
    pub fn umm_root(&self) -> [u8; 0] {
        {
            u8::default();
            [] as [u8; 0]
        } // umm_root is always empty (not even present in json-rpc)
    }
}

#[must_use]
pub fn encode_list(rlp_list: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let payload_length: usize = rlp_list.iter().map(Vec::len).sum();
    alloy_rlp::Header {
        list: true,
        payload_length,
    }
    .encode(&mut out);
    for field in rlp_list {
        out.extend_from_slice(&field);
    }

    out
}

pub fn deserialize_hex_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(s, 16).map_err(serde::de::Error::custom)
}

pub fn deserialize_hex_h256<'de, D>(deserializer: D) -> Result<H256, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s);
    let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
    Ok(H256::from_slice(&bytes))
}

pub fn deserialize_hex_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s);
    hex::decode(s).map_err(serde::de::Error::custom)
}

pub fn deserialize_hex_u256<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s);
    U256::from_str_radix(s, 16).map_err(serde::de::Error::custom)
}

pub fn deserialize_hex_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s);
    u32::from_str_radix(s, 16).map_err(serde::de::Error::custom)
}

pub fn deserialize_vec_hex_h256<'de, D>(deserializer: D) -> Result<Vec<H256>, D::Error>
where
    D: Deserializer<'de>,
{
    let strings: Vec<String> = Deserialize::deserialize(deserializer)?;
    strings
        .iter()
        .map(|s| {
            let s = s.strip_prefix("0x").unwrap_or(s);
            let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
            Ok(H256::from_slice(&bytes))
        })
        .collect()
}

pub fn deserialize_hex_bytes_20<'de, D>(deserializer: D) -> Result<[u8; 20], D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s);
    let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
    if bytes.len() != 20 {
        return Err(serde::de::Error::custom(format!(
            "expected 20 bytes, got {}",
            bytes.len()
        )));
    }
    let mut array = [0u8; 20];
    array.copy_from_slice(&bytes);
    Ok(array)
}

pub fn deserialize_hex_u256_option<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(s) => {
            let s = s.strip_prefix("0x").unwrap_or(&s);
            U256::from_str_radix(s, 16)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
        None => Ok(None),
    }
}

impl fmt::Debug for RskBlockHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let short = |h: &H256| {
            let hex = hex::encode(h);
            format!("0x{}…{}", &hex[..8], &hex[hex.len().saturating_sub(4)..])
        };

        write!(
            f,
            "RskBlockHeader {{ number: {}, hash: {}, parent: {}, diff: {}, ts: {}, uncles_hash: {}, coinbase: 0x{}, state_root: {}, tx_root: {}, receipt_root: {}, logs_bloom: {} bytes, gas_limit: 0x{}, gas_used: {}, extra_data: {} bytes, paid_fees: {}, min_gas_price: {:?}, uncle_count: {}, umm_root: {:?}, mm_header: {} bytes }}",
            self.number,
            short(&self.hash),
            short(&self.parent),
            self.difficulty,
            self.timestamp,
            short(&self.uncles_hash),
            hex::encode(self.coinbase),
            short(&self.state_root),
            short(&self.tx_trie_root),
            short(&self.receipt_trie_root),
            self.logs_bloom.len(),
            hex::encode(&self.gas_limit),
            self.gas_used,
            self.extra_data.len(),
            self.paid_fees,
            self.minimum_gas_price,
            self.uncles.len(),
            self.umm_root(),
            self.bitcoin_merged_mining_header.len(),
        )
    }
}
