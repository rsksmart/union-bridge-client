use std::fs;
use std::str::FromStr;

use check_fork_tester::TesterRskBlockHeader;
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::block_header::RskBlockHeader;
use crate::{
    CheckForkArgs, RskBlock, SUPERBLOCK_TIMES_DIFFICULTY, build_check_fork_journal_from_args,
    check_fork, compute_pegout_id,
};

const DEFAULT_DIFFICULTY: u64 = 1_000_000;
const DEFAULT_BLOCK_COUNT: u32 = 4;
const DEFAULT_VERSION: u8 = 1;
const DEFAULT_SEQ_ID: u32 = 1;
const DEFAULT_RAND: u32 = 0xA2C0_FFEE;
const DEFAULT_STREAM_ID: u32 = 1;
const DEFAULT_PACKET_ID: u32 = 1;
const DEFAULT_UTXO_ID: u32 = 4;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TestCaseBlockHashValidation {
    pub header: TesterRskBlockHeader,
    #[serde(rename = "expectedHash")]
    pub expected_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TestCaseMiniChainHashValidation {
    pub chain: Vec<TestCaseBlockHashValidation>,
}

#[test]
fn succeeds_for_valid_a2_fixture() {
    let args = build_valid_args(DEFAULT_BLOCK_COUNT);

    let result = check_fork(&args);

    assert_eq!(result, Ok(total_effort(&args.block_list)));
}

#[test]
fn compute_pegout_id_matches_a2_encoding() {
    let args = build_valid_args(DEFAULT_BLOCK_COUNT);

    let mut hasher = Keccak256::new();
    hasher.update([args.version]);
    hasher.update(args.seq_id.to_be_bytes());
    hasher.update(args.stream_id.to_be_bytes());
    hasher.update(args.packet_id.to_be_bytes());
    hasher.update(args.utxo_id.to_be_bytes());
    hasher.update(args.operator_id);
    hasher.update(args.rand.to_be_bytes());
    let expected = H256::from_slice(&hasher.finalize());

    assert_eq!(compute_pegout_id(&args), expected);
}

#[test]
fn journal_layout_is_exactly_72_bytes() {
    let args = build_valid_args(DEFAULT_BLOCK_COUNT);
    let journal = build_check_fork_journal_from_args(&args, true).to_bytes();
    let pegout_id = compute_pegout_id(&args);

    assert_eq!(journal.len(), 72);
    assert_eq!(&journal[..32], &args.operator_id);
    assert_eq!(journal[32], 0);
    assert_eq!(&journal[33..65], pegout_id.as_bytes());
    assert_eq!(&journal[65..69], &args.utxo_id.to_be_bytes());
    assert_eq!(journal[69], 1);
    assert_eq!(&journal[70..72], &u16::from(args.version).to_be_bytes());
}

#[test]
fn fails_when_block_list_has_less_than_two_blocks() {
    let args = build_valid_args(1);

    let result = check_fork(&args);

    assert_eq!(result, Err("A2 requires at least two blocks"));
}

#[test]
fn fails_when_first_block_contains_the_pegout_event() {
    let mut args = build_valid_args(DEFAULT_BLOCK_COUNT);
    let pegout_id = compute_pegout_id(&args);
    let first_block = &mut args.block_list[0];
    first_block.header.version = 2;
    first_block.header.base_event = Some(pegout_id.as_bytes().to_vec());
    rehash(first_block);

    let result = check_fork(&args);

    assert_eq!(result, Err("First block must not contain the PegOutID base event"));
}

#[test]
fn fails_when_continuation_block_is_missing_the_pegout_event() {
    let mut args = build_valid_args(DEFAULT_BLOCK_COUNT);
    let block = &mut args.block_list[2];
    block.header.version = 1;
    block.header.base_event = None;
    rehash(block);

    let result = check_fork(&args);

    assert_eq!(result, Err("A2 continuation block is missing the expected PegOutID base event"));
}

#[test]
fn fails_when_base_event_exists_but_header_version_is_not_v2() {
    let mut args = build_valid_args(DEFAULT_BLOCK_COUNT);
    let block = &mut args.block_list[2];
    block.header.version = 1;
    rehash(block);

    let result = check_fork(&args);

    assert_eq!(result, Err("Block with base event must use header version 2"));
}

#[test]
fn fails_when_blocks_are_not_consecutive() {
    let mut args = build_valid_args(2);
    args.block_list[1].header.number = args.block_list[0].header.number + 2;

    let result = check_fork(&args);

    assert_eq!(result, Err("Block numbers are not consecutive"));
}

#[test]
fn fails_when_consecutive_blocks_are_not_parent_child() {
    let mut args = build_valid_args(2);
    args.block_list[1].header.parent = H256::from_low_u64_be(1);

    let result = check_fork(&args);

    assert_eq!(result, Err("Invalid parent linkage between blocks"));
}

#[test]
fn fails_when_consecutive_block_timestamp_is_not_increasing() {
    let mut args = build_valid_args(2);
    args.block_list[1].header.timestamp = args.block_list[0].header.timestamp;

    let result = check_fork(&args);

    assert_eq!(result, Err("Block Timestamp is not increasing"));
}

#[test]
fn fails_when_consecutive_block_difficulty_is_lower_than_bounds() {
    let mut args = build_valid_args(2);
    let first_difficulty = args.block_list[0].header.difficulty;
    args.block_list[1].header.difficulty =
        first_difficulty.saturating_sub(first_difficulty / 399);

    let result = check_fork(&args);

    assert_eq!(result, Err("Consecutive Block difficulty is out of bounds"));
}

#[test]
fn fails_when_consecutive_block_difficulty_is_higher_than_bounds() {
    let mut args = build_valid_args(2);
    let first_difficulty = args.block_list[0].header.difficulty;
    args.block_list[1].header.difficulty =
        first_difficulty.saturating_add(first_difficulty / 399);

    let result = check_fork(&args);

    assert_eq!(result, Err("Consecutive Block difficulty is out of bounds"));
}

#[test]
fn block_hash_changes_when_base_event_changes() {
    let args = build_valid_args(DEFAULT_BLOCK_COUNT);
    let mut lhs = args.block_list[2].header.clone();
    let mut rhs = args.block_list[2].header.clone();

    lhs.base_event = Some([0x11; 32].to_vec());
    lhs.version = 2;
    lhs.hash = lhs.calculate_block_hash().expect("lhs hash");

    rhs.base_event = Some([0x22; 32].to_vec());
    rhs.version = 2;
    rhs.hash = rhs.calculate_block_hash().expect("rhs hash");

    assert_ne!(lhs.hash, rhs.hash);
}

#[test]
fn succeed_if_block_hash_eq_expected_hash() {
    let test_case = serde_json::from_slice::<TestCaseBlockHashValidation>(
        &fs::read("src/tests/block-regtest-min-gas-price-zero.json").expect("fixture"),
    )
    .expect("test case");

    let header = header_from_tester(&test_case.header);
    let hash = header.calculate_block_hash().expect("calculate hash");
    let expected_hash = H256::from_str(&test_case.expected_hash).expect("expected hash");

    assert_eq!(expected_hash, hash);
    assert_eq!(test_case.header.hash, hash);
}

#[test]
fn succeed_if_minichain_hashes_are_valid() {
    assert_minichain_hashes_are_valid_from_fixture("src/tests/blockhash-mini-chain.json");
}

#[test]
fn succeed_if_testnet_minichain_hashes_are_valid() {
    assert_minichain_hashes_are_valid_from_fixture("src/tests/blockhash-mini-chain-testnet.json");
}

fn build_valid_args(block_count: u32) -> CheckForkArgs {
    let operator_id = [0x11; 32];
    let mut args = CheckForkArgs {
        version: DEFAULT_VERSION,
        seq_id: DEFAULT_SEQ_ID,
        rand: DEFAULT_RAND,
        stream_id: DEFAULT_STREAM_ID,
        packet_id: DEFAULT_PACKET_ID,
        utxo_id: DEFAULT_UTXO_ID,
        operator_id,
        init_block_time: 1_700_000_000,
        init_block_number: 1_000,
        required_effort: U256::zero(),
        required_num_blocks: block_count.max(1),
        block_list: Vec::new(),
    };

    let pegout_id = compute_pegout_id(&args);
    let pow = calculate_superblock_pow(U256::from(DEFAULT_DIFFICULTY));
    let mut parent_hash = H256::from_low_u64_be(0xABCD_EF01);

    for index in 0..block_count {
        let number = args.init_block_number + 1 + u64::from(index);
        let timestamp = args.init_block_time + 1 + u64::from(index);
        let base_event = if index >= 2 { Some(pegout_id.as_bytes().to_vec()) } else { None };
        let header = build_header(
            number,
            timestamp,
            parent_hash,
            U256::from(DEFAULT_DIFFICULTY),
            base_event,
        );
        parent_hash = header.hash;
        args.block_list.push(RskBlock { uncles: Vec::new(), pow, header });
    }

    args.required_effort = total_effort(&args.block_list);
    args
}

fn build_header(
    number: u64,
    timestamp: u64,
    parent: H256,
    difficulty: U256,
    base_event: Option<Vec<u8>>,
) -> RskBlockHeader {
    let mut header = RskBlockHeader {
        version: if base_event.is_some() { 2 } else { 1 },
        number,
        parent,
        difficulty,
        timestamp,
        gas_limit: vec![0x01],
        minimum_gas_price: Some(U256::zero()),
        base_event,
        ..RskBlockHeader::default()
    };
    header.hash = header.calculate_block_hash().expect("header hash");
    header
}

fn total_effort(blocks: &[RskBlock]) -> U256 {
    blocks.iter().fold(U256::zero(), |acc, block| {
        acc.checked_add(block_effort(block.pow)).expect("effort overflow")
    })
}

fn calculate_superblock_pow(difficulty: U256) -> H256 {
    let target_effort = difficulty
        .checked_mul(U256::from(SUPERBLOCK_TIMES_DIFFICULTY))
        .expect("difficulty overflow");
    let pow = U256::MAX.checked_div(target_effort).expect("division by zero computing pow");
    H256::from(pow.to_big_endian())
}

fn block_effort(pow: H256) -> U256 {
    let pow = U256::from_big_endian(pow.as_bytes());
    U256::MAX.checked_div(pow).expect("division by zero on block effort")
}

fn rehash(block: &mut RskBlock) {
    block.header.hash = block.header.calculate_block_hash().expect("rehash block");
}

fn assert_minichain_hashes_are_valid_from_fixture(path: &str) {
    let test_cases =
        serde_json::from_slice::<TestCaseMiniChainHashValidation>(&fs::read(path).expect("fixture"))
            .expect("test cases");

    for (index, block) in test_cases.chain.iter().enumerate() {
        let header = header_from_tester(&block.header);
        let calculated_hash = header.calculate_block_hash().expect("calculate hash");
        let expected_hash = H256::from_str(&block.expected_hash).expect("expected hash");

        assert_eq!(
            calculated_hash, header.hash,
            "Block hash mismatch at index {index} (height {})",
            header.number
        );
        assert_eq!(
            calculated_hash, expected_hash,
            "Block hash mismatch with expectedHash at index {index} (height {})",
            header.number
        );
    }
}

fn header_from_tester(t: &TesterRskBlockHeader) -> RskBlockHeader {
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
