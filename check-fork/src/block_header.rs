#![allow(clippy::missing_errors_doc)]

use primitive_types::H256;
use primitive_types::U256;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use sha3::Digest;
use sha3::Keccak256;
use std::fmt;

use crate::BridgeEvent;

// todo(fede) try to unify this struct with types.RskBlock
// todo(fede) reconsider all the serde anotations
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Block {
    pub number: u64,
    pub hash: H256,
    pub parent: H256,
    pub difficulty: U256,
    pub timestamp: u64,
    pub bridge_event: Option<BridgeEvent>,
    #[serde(default)]
    pub uncles: Vec<Block>,
    // alternatively we can receive `bitcoinMergedMiningHeader`, but we would need to include bitcoin crate here, etc.
    pub pow: H256,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestCaseBlockHashValidation {
    pub header: RskBlockHeader,
    #[serde(rename = "expectedHash")]
    pub expected_hash: String,
}

// todo(fede) currently we are not taking into account ummRoot field becaue it's always empty and
// discarded in the rskj rlp encoding
#[derive(Serialize, Deserialize, Clone)]
pub struct RskBlockHeader {
    #[serde(rename = "number", deserialize_with = "deserialize_hex_u64")]
    pub number: u64, // Block height (genesis = 0)
    #[serde(skip)]
    pub hash: H256, // Keccak-256 of the encoded header
    #[serde(rename = "parentHash", deserialize_with = "deserialize_hex_h256")]
    pub parent: H256, // Keccak-256 hash of the parent block
    #[serde(rename = "difficulty", deserialize_with = "deserialize_hex_u256")]
    pub difficulty: U256, // Target difficulty for this block
    #[serde(rename = "timestamp", deserialize_with = "deserialize_hex_u64")]
    pub timestamp: u64, // Unix time (seconds) when the block was created
    // #[serde(rename = "unclesHash", deserialize_with = "deserialize_hex_h256")]
    #[serde(rename = "sha3Uncles", deserialize_with = "deserialize_hex_h256")]
    pub uncles_hash: H256, // SHA3-256 hash of the uncles list portion
    // #[serde(rename = "coinbase", deserialize_with = "deserialize_hex_bytes_20")]
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
    pub gas_limit: Vec<u8>, // Current limit of gas expenditure per block (bytes, not u64)
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
    pub minimum_gas_price: Option<U256>, // Minimum gas price for a tx to be included (Coin, can be null)
    // #[serde(rename = "uncleCount", deserialize_with = "deserialize_hex_u32")]
    #[serde(rename = "uncles", deserialize_with = "deserialize_hex_uncle_count")]
    pub uncle_count: u32, // Number of uncles in the block

    // Merged mining fields
    #[serde(
        rename = "bitcoinMergedMiningHeader",
        deserialize_with = "deserialize_hex_bytes"
    )]
    pub bitcoin_merged_mining_header: Vec<u8>, // 80-byte Bitcoin block header for merged mining
    #[serde(
        rename = "bitcoinMergedMiningMerkleProof",
        deserialize_with = "deserialize_hex_bytes"
    )]
    pub bitcoin_merged_mining_merkle_proof: Vec<u8>, // Bitcoin merkle proof of coinbase tx
    #[serde(
        rename = "bitcoinMergedMiningCoinbaseTransaction",
        deserialize_with = "deserialize_hex_bytes"
    )]
    pub bitcoin_merged_mining_coinbase_transaction: Vec<u8>, // Bitcoin protobuf serialized coinbase tx (compressed)

                                                             // follwoing fields will be included in the next hardfork (reed)
                                                             // #[serde(rename = "ummRoot", deserialize_with = "deserialize_allways_empty_vec")]
                                                             // pub umm_root: [u8; 20], // UMM root (only if block is UMM, must be exactly 20 bytes)
                                                             // Extension fields (RSKIP-351)
                                                             // pub version: u8,                                   // Header version
                                                             // pub tx_execution_sublists_edges: Option<Vec<u16>>, // Edges of transaction execution sublists
}

impl fmt::Debug for RskBlockHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let short = |h: &H256| {
            let hex = hex::encode(h);
            format!("0x{}…{}", &hex[..8], &hex[hex.len().saturating_sub(4)..])
        };

        write!(
            f,
            "TmpRskBlockHeader {{ number: {}, hash: {}, parent: {}, diff: {}, ts: {}, uncles_hash: {}, coinbase: 0x{}, state_root: {}, tx_root: {}, receipt_root: {}, logs_bloom: {} bytes, gas_limit: 0x{}, gas_used: {}, extra_data: {} bytes, paid_fees: {}, min_gas_price: {:?}, uncle_count: {}, mm_header: {} bytes, mm_merkle_proof: {} bytes, mm_coinbase: {} bytes }}",
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
            self.uncle_count,
            self.bitcoin_merged_mining_header.len(),
            self.bitcoin_merged_mining_merkle_proof.len(),
            self.bitcoin_merged_mining_coinbase_transaction.len()
        )
    }
}

impl RskBlockHeader {
    pub fn uncle_count(&self) -> u32 {
        todo!("this needs to be calculated ")
    }

    pub fn umm_root(&self) -> Vec<u8> {
        // RSKJ treats ummRoot as "absent" by using an empty element when the
        // field is not set (pre-UMM blocks). We mirror that behaviour instead
        // of injecting 20 zeroed bytes, which changes the RLP.
        Vec::new()
    }

    pub fn calculate_block_hash(&self) -> Result<H256, &'static str> {
        let rlp_encoded: Vec<u8> = self.encode_rlp()?;
        let mut hasher = Keccak256::new();
        hasher.update(&rlp_encoded);
        let block_hash = H256::from_slice(&hasher.finalize());

        println!("Block hash: {block_hash}");

        Ok(block_hash)
    }

    pub fn encode_rlp(&self) -> Result<Vec<u8>, &'static str> {
        // todo(fede) do i need to perform any other kind of validation befeore encoding?
        let Some(minimum_gas_price) = self.minimum_gas_price else {
            return Err("minimum_gas_price is None");
        };
        let mut encoded_fields: Vec<Vec<u8>> = vec![
            encode_h256("parent", &self.parent),
            encode_h256("uncles_hash", &self.uncles_hash),
            encode_bytes("coinbase", self.coinbase.as_slice()),
            encode_h256("state_root", &self.state_root),
            encode_h256("tx_trie_root", &self.tx_trie_root),
            encode_h256("receipt_trie_root", &self.receipt_trie_root),
            encode_bytes("logs_bloom", self.logs_bloom.as_slice()),
            encode_coin_value("difficulty", &self.difficulty),
            encode_u64_value("number", self.number),
            encode_bytes("gas_limit", self.gas_limit.as_slice()),
            encode_u64_value("gas_used", self.gas_used),
            encode_u64_value("timestamp", self.timestamp),
            encode_bytes("extra_data", self.extra_data.as_slice()),
            encode_coin_value("paid_fees", &self.paid_fees),
            encode_signed_coin_value("minimum_gas_price", &minimum_gas_price),
            encode_u32_value("uncle_count", self.uncle_count),
            encode_bytes("umm_root", &self.umm_root()),
        ];

        encoded_fields.push(encode_bytes(
            "bitcoin_merged_mining_header",
            self.bitcoin_merged_mining_header.as_slice(),
        ));

        // encoded_fields.push(encode_bytes(
        //     "bitcoin_merged_mining_merkle_proof",
        //     self.bitcoin_merged_mining_merkle_proof.as_slice(),
        // ));
        // encoded_fields.push(encode_bytes(
        //     "bitcoin_merged_mining_coinbase_transaction",
        //     self.bitcoin_merged_mining_coinbase_transaction.as_slice(),
        // ));

        let out = encode_list(encoded_fields);

        Ok(out)
    }

    fn _get_logs_bloom_field(&self) -> H256 {
        todo!("wip")
    }
}

#[must_use]
// todo(fede) this fn is unecessary, remove
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

fn encode_h256(label: &str, value: &H256) -> Vec<u8> {
    let v = alloy_rlp::encode(value.as_bytes());
    println!("RLP encode {label}: 0x{}", hex::encode(&v));
    v
}

fn encode_bytes(label: &str, value: &[u8]) -> Vec<u8> {
    let v = alloy_rlp::encode(value);
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

fn encode_u64_value(label: &str, value: u64) -> Vec<u8> {
    let v = alloy_rlp::encode(value);
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

fn encode_u32_value(label: &str, value: u32) -> Vec<u8> {
    let v = alloy_rlp::encode(value);
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

fn encode_coin_value(label: &str, value: &U256) -> Vec<u8> {
    let v = alloy_rlp::encode(u256_be_coin_bytes(value).as_slice());
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

fn encode_signed_coin_value(label: &str, value: &U256) -> Vec<u8> {
    // RLP integers are big-endian: 0 -> 0x80, 0x00–0x7f encode as-is,
    // and if the MSB≥0x80 we prefix 0x00 to keep the value positive
    // before adding the length prefix.
    let mut bytes = u256_be_coin_bytes(value);
    if bytes.first().is_some_and(|b| *b >= 0x80) {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0); // we add a "0x00" prefix to keep the value positive
        prefixed.extend_from_slice(&bytes);
        bytes = prefixed;
    }
    let v = alloy_rlp::encode(bytes.as_slice());
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

fn u256_be_trimmed(value: &U256) -> Vec<u8> {
    // positive integers must be represented in big-endian binary form with
    // no leading zeroes (thus making the integer value zero equivalent to
    // the empty byte array). Deserialized positive integers with leading
    // zeroes must be treated as invalid by any higher-order protocol using RLP.
    // we inherit this from ethereum, if any doubts checkout the ethereum yellow paper.
    let buf = value.to_big_endian();
    let first_non_zero = buf.iter().position(|&b| b != 0).unwrap_or(buf.len());
    match first_non_zero {
        idx if idx == buf.len() => Vec::new(),
        idx => buf[idx..].to_vec(),
    }
}

fn u256_be_coin_bytes(value: &U256) -> Vec<u8> {
    // RSKJ encodes coin values using RLP's empty element for zero amounts,
    // not a single 0x00 byte. Returning an empty vec reproduces the same
    // `0x80` encoding for zero and trims leading zeroes otherwise.
    if value.is_zero() {
        return Vec::new();
    }
    u256_be_trimmed(value)
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
    H256::from_slice(&bytes);
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

pub fn deserialize_hex_uncle_count<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Vec<String> = Deserialize::deserialize(deserializer)?;
    if s.len() != 1 {
        return Err(serde::de::Error::custom(format!(
            "expected 1 element, got {}",
            s.len()
        )));
    }

    Ok(s.len() as u32)
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

pub fn deserialize_allways_empty_vec<'de, D>(deserializer: D) -> Result<[u8; 20], D::Error>
where
    D: Deserializer<'de>,
{
    // Consume whatever comes (string, bytes, null, etc.) and ignore it.
    let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
    Ok([0u8; 20])
}
