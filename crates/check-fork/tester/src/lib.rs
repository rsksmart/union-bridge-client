use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use bitcoin::blockdata::block::Header;
use bitcoin::consensus::encode::deserialize as btc_deserialize;
use check_fork::block_header::{
    RskBlockHeader, deserialize_hex_bytes, deserialize_hex_bytes_20, deserialize_hex_h256,
    deserialize_hex_u64, deserialize_hex_u256, deserialize_hex_u256_option,
    deserialize_vec_hex_h256,
};
use check_fork::{CheckForkArgs, RskBlock, build_check_fork_journal_from_args, compute_pegout_id};
use primitive_types::{H256, U256};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const RSK_RPC_URL: &str = "https://public-node.rsk.co";
const SUPERBLOCK_THRESHOLD_FACTOR: u64 = 20;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TesterRskBlockHeader {
    #[serde(rename = "number", deserialize_with = "deserialize_hex_u64")]
    pub number: u64,
    #[serde(rename = "hash", deserialize_with = "deserialize_hex_h256")]
    pub hash: H256,
    #[serde(rename = "parentHash", deserialize_with = "deserialize_hex_h256")]
    pub parent: H256,
    #[serde(rename = "difficulty", deserialize_with = "deserialize_hex_u256")]
    pub difficulty: U256,
    #[serde(rename = "timestamp", deserialize_with = "deserialize_hex_u64")]
    pub timestamp: u64,
    #[serde(rename = "sha3Uncles", deserialize_with = "deserialize_hex_h256")]
    pub uncles_hash: H256,
    #[serde(rename = "miner", deserialize_with = "deserialize_hex_bytes_20")]
    pub coinbase: [u8; 20],
    #[serde(rename = "stateRoot", deserialize_with = "deserialize_hex_h256")]
    pub state_root: H256,
    #[serde(rename = "transactionsRoot", deserialize_with = "deserialize_hex_h256")]
    pub tx_trie_root: H256,
    #[serde(rename = "receiptsRoot", deserialize_with = "deserialize_hex_h256")]
    pub receipt_trie_root: H256,
    // This is the json-rpc logsBloom field.
    // check-fork derives the compressed v1 extension data from it when hashing headers.
    #[serde(rename = "logsBloom", deserialize_with = "deserialize_hex_bytes")]
    pub extension_data: Vec<u8>,
    #[serde(rename = "gasLimit", deserialize_with = "deserialize_hex_bytes")]
    pub gas_limit: Vec<u8>,
    #[serde(rename = "gasUsed", deserialize_with = "deserialize_hex_u64")]
    pub gas_used: u64,
    #[serde(rename = "extraData", deserialize_with = "deserialize_hex_bytes")]
    pub extra_data: Vec<u8>,
    #[serde(rename = "paidFees", deserialize_with = "deserialize_hex_u256")]
    pub paid_fees: U256,
    #[serde(rename = "minimumGasPrice", deserialize_with = "deserialize_hex_u256_option")]
    pub minimum_gas_price: Option<U256>,
    #[serde(rename = "rskPteEdges", default, deserialize_with = "deserialize_optional_u16_vec")]
    pub rsk_pte_edges: Option<Vec<u16>>,
    #[serde(rename = "uncles", deserialize_with = "deserialize_vec_hex_h256", default)]
    pub uncles: Vec<H256>,
    #[serde(rename = "bitcoinMergedMiningHeader", deserialize_with = "deserialize_hex_bytes")]
    pub bitcoin_merged_mining_header: Vec<u8>,
}

impl From<&TesterRskBlockHeader> for RskBlockHeader {
    fn from(t: &TesterRskBlockHeader) -> Self {
        RskBlockHeader {
            version: 1,
            number: t.number,
            hash: t.hash,
            parent: t.parent,
            difficulty: t.difficulty,
            timestamp: t.timestamp,
            uncles_hash: t.uncles_hash,
            coinbase: t.coinbase,
            state_root: t.state_root,
            tx_trie_root: t.tx_trie_root,
            receipt_trie_root: t.receipt_trie_root,
            extension_data: t.extension_data.clone(),
            gas_limit: t.gas_limit.clone(),
            gas_used: t.gas_used,
            extra_data: t.extra_data.clone(),
            paid_fees: t.paid_fees,
            minimum_gas_price: t.minimum_gas_price,
            uncles: t.uncles.clone(),
            rsk_pte_edges: t.rsk_pte_edges.clone(),
            base_event: None,
            bitcoin_merged_mining_header: t.bitcoin_merged_mining_header.clone(),
        }
    }
}

// Used mainly for deserialization and also to avoid adding
// bitcoin-specific RPC dependencies to the `check-fork` crate.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TesterRskBlock {
    #[serde(flatten)]
    header: TesterRskBlockHeader,
    #[serde(skip)]
    uncles: Vec<TesterRskBlock>, // this field should be filled later
}

fn deserialize_optional_u16_vec<'de, D>(deserializer: D) -> Result<Option<Vec<u16>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<u16>>::deserialize(deserializer)
}

impl From<&TesterRskBlock> for RskBlock {
    fn from(tester_block: &TesterRskBlock) -> Self {
        RskBlock {
            uncles: tester_block.uncles.iter().map(RskBlock::from).collect(),
            pow: tester_block.pow().expect("pow is not valid"),
            header: RskBlockHeader::from(&tester_block.header),
        }
    }
}

impl TesterRskBlock {
    fn pow(&self) -> Result<H256, Box<dyn Error>> {
        if self.header.bitcoin_merged_mining_header.len() != 80 {
            return Err("bitcoin_merged_mining_header is not 80 bytes long".into());
        }
        let btc_header: Header = btc_deserialize(&self.header.bitcoin_merged_mining_header)
            .map_err(|e| {
                format!(
                    "Failed to deserialize btc header: {e:?}, data: {:?}",
                    self.header.bitcoin_merged_mining_header
                )
            })?;
        Ok(H256::from_str(&btc_header.block_hash().to_string())?)
    }

    fn add_uncle(&mut self, uncle: TesterRskBlock) {
        self.uncles.push(uncle);
    }
}

pub async fn get_blocks(
    start_block_number: u64,
    num_of_blocks: u32,
    log_super_block: bool,
) -> Result<Vec<RskBlock>, Box<dyn Error>> {
    // Disable system proxy autodetection: on macOS runners this can panic before the RPC call.
    let client = Client::builder().no_proxy().build()?;
    let mut blocks = vec![];

    for i in 0..num_of_blocks {
        fetch_block_by_num(start_block_number, log_super_block, &client, &mut blocks, i).await?;
    }

    let mut blocks_with_uncles = Vec::with_capacity(blocks.len());
    for block in blocks {
        let uncles_hashes = block.header.uncles.clone();
        let fetched = fetch_uncles(&client, uncles_hashes, block).await;
        blocks_with_uncles.push(fetched);
    }

    Ok(blocks_with_uncles.iter().map(RskBlock::from).collect())
}

pub fn parse_operator_id_hex(value: &str) -> Result<[u8; 32], Box<dyn Error>> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(trimmed)?;
    if bytes.len() != 32 {
        return Err(format!("operator_id must be exactly 32 bytes, got {}", bytes.len()).into());
    }

    let mut operator_id = [0u8; 32];
    operator_id.copy_from_slice(&bytes);
    Ok(operator_id)
}

pub fn apply_a2_base_event(blocks: &mut [RskBlock], pegout_id: H256) -> Result<(), Box<dyn Error>> {
    if blocks.len() < 2 {
        return Err("A2 requires at least two blocks".into());
    }

    for block in blocks.iter_mut() {
        block.uncles.clear();
    }

    for index in 0..blocks.len() {
        if index >= 2 {
            blocks[index].header.version = 2;
            blocks[index].header.base_event = Some(pegout_id.as_bytes().to_vec());
            blocks[index].header.parent = blocks[index - 1].header.hash;
            blocks[index].header.hash =
                blocks[index].header.calculate_block_hash().map_err(std::io::Error::other)?;
        } else {
            blocks[index].header.version = 1;
            blocks[index].header.base_event = None;
        }
    }

    Ok(())
}

pub fn calculate_total_effort(blocks: &[RskBlock]) -> Result<U256, Box<dyn Error>> {
    let mut total = U256::zero();
    for block in blocks {
        total = add_block_effort(total, block)?;
        for uncle in &block.uncles {
            total = add_block_effort(total, uncle)?;
        }
    }
    Ok(total)
}

pub fn write_a2_artifacts(output_dir: &Path, args: &CheckForkArgs) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(output_dir)?;

    let pegout_id = compute_pegout_id(args);
    let expected_journal = build_check_fork_journal_from_args(args, true).to_bytes();
    fs::write(output_dir.join("expected_journal.bin"), expected_journal)?;
    fs::write(output_dir.join("expected_journal.hex"), hex::encode(expected_journal))?;
    fs::write(
        output_dir.join("expected_journal.csv"),
        expected_journal.iter().map(u8::to_string).collect::<Vec<_>>().join(","),
    )?;
    fs::write(output_dir.join("computed_pegout_id.hex"), hex::encode(pegout_id.as_bytes()))?;
    fs::write(output_dir.join("fixture_summary.txt"), build_summary(args, pegout_id))?;

    Ok(())
}

async fn fetch_uncles(
    client: &Client,
    uncles_hashes: Vec<H256>,
    mut block: TesterRskBlock,
) -> TesterRskBlock {
    for uncle_hash in uncles_hashes {
        let uncle = fetch_block_by_hash(uncle_hash, client).await.expect("Failed to fetch uncle");
        block.add_uncle(uncle);
    }
    block
}

async fn fetch_block_by_hash(
    hash: H256,
    client: &Client,
) -> Result<TesterRskBlock, Box<dyn Error>> {
    let hash_hex = format!("0x{hash:x}");
    let request_body = json!({
        "jsonrpc": "2.0",
        "method": "eth_getBlockByHash",
        "params": [hash_hex, true],
        "id": 1,
    });
    let response = client.post(RSK_RPC_URL).json(&request_body).send().await?;
    let response_json: Value = response.json().await?;
    if response_json.get("error").is_some() {
        return Err(format!("Error fetching block by hash {hash_hex}: {response_json:?}").into());
    }
    let Some(result) = response_json.get("result") else {
        return Err(format!("No result for block hash {hash_hex}").into());
    };
    Ok(serde_json::from_str(&result.to_string())?)
}

async fn fetch_block_by_num(
    start_block_number: u64,
    log_super_block: bool,
    client: &Client,
    blocks: &mut Vec<TesterRskBlock>,
    num: u32,
) -> Result<(), Box<dyn Error>> {
    let block_number_hex = format!("{:#x}", start_block_number + u64::from(num));
    let request_body = json!({
        "jsonrpc": "2.0",
        "method": "eth_getBlockByNumber",
        "params": [block_number_hex, true],
        "id": 1,
    });
    let response = client.post(RSK_RPC_URL).json(&request_body).send().await?;
    let response_json: Value = response.json().await?;
    if response_json.get("error").is_some() {
        println!(
            "Error fetching block {}: {:?}",
            start_block_number + u64::from(num),
            response_json
        );
    } else if let Some(result) = response_json.get("result") {
        let block: TesterRskBlock = serde_json::from_str(&result.to_string())?;
        if log_super_block {
            log_if_superblock(&block)?;
        }
        blocks.push(block);
    }
    Ok(())
}

fn log_if_superblock(block: &TesterRskBlock) -> Result<(), Box<dyn Error>> {
    // Parse the block's actual PoW from `bitcoinMergedMiningHeader`.
    let actual_block_pow =
        U256::from_big_endian(block.header.bitcoin_merged_mining_header.as_slice());
    // Compute the PoW target from difficulty by inversion. `difficulty 1` maps to `U256::MAX`.
    let target_block_pow =
        U256::MAX.checked_div(block.header.difficulty).ok_or("0 division on log_if_superblock")?;
    // Define a superblock as one whose PoW is at least N times harder than the required target.
    let superblock_pow = target_block_pow / SUPERBLOCK_THRESHOLD_FACTOR;

    // If the actual block PoW is lower (i.e. harder) than the superblock threshold, we found one.
    if actual_block_pow < superblock_pow {
        let timestamp_i64 = i64::try_from(block.header.timestamp).unwrap_or(i64::MAX);
        let formatted_time =
            chrono::DateTime::from_timestamp(timestamp_i64, 0).unwrap().format("%Y-%m-%d %H:%M:%S");

        println!(
            "SuperBlock: {}, pow: {:?}, threshold: 0x{:064x}, time: {}",
            block.header.number,
            &block.header.bitcoin_merged_mining_header,
            superblock_pow,
            formatted_time
        );
    }

    Ok(())
}

fn add_block_effort(total: U256, block: &RskBlock) -> Result<U256, Box<dyn Error>> {
    let pow = U256::from_big_endian(block.pow.as_bytes());
    let effort = U256::MAX.checked_div(pow).ok_or("division by zero on block effort")?;
    total.checked_add(effort).ok_or_else(|| "effort overflow".into())
}

fn build_summary(args: &CheckForkArgs, pegout_id: H256) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "version={}", args.version);
    let _ = writeln!(out, "seq_id={}", args.seq_id);
    let _ = writeln!(out, "rand=0x{:08x}", args.rand);
    let _ = writeln!(out, "stream_id={}", args.stream_id);
    let _ = writeln!(out, "packet_id={}", args.packet_id);
    let _ = writeln!(out, "utxo_id={}", args.utxo_id);
    let _ = writeln!(out, "operator_id=0x{}", hex::encode(args.operator_id));
    let _ = writeln!(out, "pegout_id=0x{}", hex::encode(pegout_id.as_bytes()));
    let _ = writeln!(out, "block_count={}", args.block_list.len());
    let _ = writeln!(out, "required_num_blocks={}", args.required_num_blocks);
    let _ = writeln!(out, "required_effort={}", args.required_effort);
    out
}
