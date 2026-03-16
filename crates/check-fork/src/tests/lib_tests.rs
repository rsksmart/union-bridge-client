use std::fs;
use std::str::FromStr;

use check_fork_tester::TesterRskBlockHeader;
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::block_header::{RskBlockHeader, encode_list};
use crate::{
    CheckForkArgs, RskBlock, SUPERBLOCK_TIMES_DIFFICULTY, build_check_fork_journal_from_args,
    check_fork, compute_pegout_id,
};

const DEFAULT_DIFFICULTY: u128 = 5_904_436_352_267_687_415_636;
const DEFAULT_TIMESTAMP: u64 = 1000;
const DEFAULT_INIT_BLOCK_NUMBER: u64 = 100;
const DEFAULT_REQ_NUMBER_OF_BLOCKS: u32 = 2;
const DEFAULT_VERSION: u8 = 1;
const DEFAULT_SEQ_ID: u32 = 1;
const DEFAULT_RAND: u32 = 0xA2C0_FFEE;
const DEFAULT_STREAM_ID: u32 = 1;
const DEFAULT_PACKET_ID: u32 = 1;
const DEFAULT_UTXO_ID: u32 = 4;
const DEFAULT_OPERATOR_ID: [u8; 32] = [0x11; 32];

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
fn succeeds_with_two_blocks_when_all_conditions_met() {
    let mut actual_effort = U256::zero();

    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    actual_effort += calculate_effort_from_pow(first_block.pow);

    let second_block = create_child_block(&first_block);
    actual_effort += calculate_effort_from_pow(second_block.pow);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).required_effort(actual_effort).build();
    let result = check_fork(&args);

    assert_eq!(result, Ok(actual_effort), "Expected to succeed for valid input");
}

#[test]
fn succeeds_with_two_blocks_and_one_uncle_when_all_conditions_met() {
    let mut actual_effort = U256::zero();

    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    actual_effort += calculate_effort_from_pow(first_block.pow);

    let second_block_uncle = create_uncle(&first_block);
    actual_effort += calculate_effort_from_pow(second_block_uncle.pow);

    let mut second_block = create_child_block(&first_block);
    second_block.uncles = vec![second_block_uncle];

    actual_effort += calculate_effort_from_pow(second_block.pow);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).required_effort(actual_effort).build();
    let result = check_fork(&args);

    assert_eq!(result, Ok(actual_effort), "Expected to succeed for valid input");
}

#[test]
fn succeeds_with_required_pegout_event_in_continuation_blocks() {
    let mut actual_effort = U256::zero();

    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    actual_effort += calculate_effort_from_pow(first_block.pow);

    let second_block = create_child_block(&first_block);
    actual_effort += calculate_effort_from_pow(second_block.pow);

    let third_block = create_child_block(&second_block);
    actual_effort += calculate_effort_from_pow(third_block.pow);

    let fourth_block = create_child_block(&third_block);
    actual_effort += calculate_effort_from_pow(fourth_block.pow);

    let block_list = vec![first_block, second_block, third_block, fourth_block];
    let mut args = CheckForkArgsBuilder::new(block_list)
        .required_num_blocks(4)
        .required_effort(actual_effort)
        .build();
    decorate_required_pegout_events(&mut args);

    let result = check_fork(&args);

    assert_eq!(result, Ok(actual_effort), "Expected to succeed for valid continuation blocks");
}

#[test]
fn compute_pegout_id_matches_public_input_encoding() {
    let args = CheckForkArgsBuilder::new(vec![
        create_first_block(DEFAULT_INIT_BLOCK_NUMBER),
        create_child_block(&create_first_block(DEFAULT_INIT_BLOCK_NUMBER)),
    ])
    .build();

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
    let args = CheckForkArgsBuilder::new(vec![
        create_first_block(DEFAULT_INIT_BLOCK_NUMBER),
        create_child_block(&create_first_block(DEFAULT_INIT_BLOCK_NUMBER)),
    ])
    .build();
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
fn fails_when_required_block_number_is_invalid() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).required_num_blocks(0).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Invalid number of required blocks"),
        "Expected to fail if requested number of blocks are invalid"
    );
}

#[test]
fn fails_when_block_list_has_less_than_two_blocks() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let block_list = vec![first_block];

    let args = CheckForkArgsBuilder::new(block_list).required_num_blocks(1).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Check-fork requires at least two blocks"),
        "Expected to fail if block_list has less than two blocks"
    );
}

#[test]
fn fails_when_provided_blocks_are_less_than_required() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).required_num_blocks(3).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Insufficient number of blocks"),
        "Expected to fail if provided blocks are less than requested"
    );
}

#[test]
fn fails_when_first_block_timestamp_is_lower_than_min_requested() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).init_block_time(1_000_000).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("First block timestamp lower than expected"),
        "Expected to fail if first block timestamp is lower than min requested"
    );
}

#[test]
fn fails_when_first_block_number_is_lower_than_min_requested() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).init_block_number(1_000_000).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("First block number lower than expected"),
        "Expected to fail if first block number is lower than min requested"
    );
}

#[test]
fn fails_when_cumulative_effort_below_expected() {
    let mut actual_effort = U256::zero();

    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    actual_effort += calculate_effort_from_pow(first_block.pow);

    let second_block_uncle = create_uncle(&first_block);
    actual_effort += calculate_effort_from_pow(second_block_uncle.pow);

    let mut second_block = create_child_block(&first_block);
    second_block.uncles = vec![second_block_uncle];
    actual_effort += calculate_effort_from_pow(second_block.pow);

    let block_list = vec![first_block, second_block];
    let expected_effort = actual_effort + 1;
    let args = CheckForkArgsBuilder::new(block_list).required_effort(expected_effort).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Cumulative PoW does not meet the required threshold"),
        "Expected to fail if cumulative PoW is lower than expected: {expected_effort}"
    );
}

#[test]
fn fails_when_blocks_are_not_consecutive() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.header.number = first_block.header.number + 2;

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Block numbers are not consecutive"),
        "Expected to fail if blocks are not consecutive"
    );
}

#[test]
fn fails_when_consecutive_blocks_are_not_parent_child() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.header.parent = H256::from_low_u64_be(1);

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Invalid parent linkage between blocks"),
        "Expected to fail if consecutive blocks are not parent-child"
    );
}

#[test]
fn fails_when_first_block_contains_the_pegout_event() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let third_block = create_child_block(&second_block);
    let fourth_block = create_child_block(&third_block);
    let block_list = vec![first_block, second_block, third_block, fourth_block];

    let mut args = CheckForkArgsBuilder::new(block_list).required_num_blocks(4).build();
    decorate_required_pegout_events(&mut args);

    let pegout_id = compute_pegout_id(&args);
    let first_block = &mut args.block_list[0];
    first_block.header.version = 2;
    first_block.header.base_event = Some(pegout_id.as_bytes().to_vec());

    let result = check_fork(&args);
    assert_eq!(result, Err("First block must not contain the PegOutID base event"));
}

#[test]
fn fails_when_continuation_block_is_missing_the_pegout_event() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let third_block = create_child_block(&second_block);
    let fourth_block = create_child_block(&third_block);
    let block_list = vec![first_block, second_block, third_block, fourth_block];

    let mut args = CheckForkArgsBuilder::new(block_list).required_num_blocks(4).build();
    decorate_required_pegout_events(&mut args);

    let block = &mut args.block_list[2];
    block.header.version = 1;
    block.header.base_event = None;

    let result = check_fork(&args);
    assert_eq!(result, Err("Continuation block is missing the expected PegOutID base event"));
}

#[test]
fn fails_when_base_event_exists_but_header_version_is_not_v2() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let third_block = create_child_block(&second_block);
    let fourth_block = create_child_block(&third_block);
    let block_list = vec![first_block, second_block, third_block, fourth_block];

    let mut args = CheckForkArgsBuilder::new(block_list).required_num_blocks(4).build();
    decorate_required_pegout_events(&mut args);

    let block = &mut args.block_list[2];
    block.header.version = 1;

    let result = check_fork(&args);
    assert_eq!(result, Err("Block with base event must use header version 2"));
}

#[test]
fn fails_when_consecutive_block_difficulty_is_lower_than_bounds() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.header.difficulty =
        first_block.header.difficulty.saturating_sub(first_block.header.difficulty / 399);
    second_block.pow = calculate_superblock_effort(second_block.header.difficulty);

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Consecutive Block difficulty is out of bounds"),
        "Expected to fail if the consecutive block difficulty is too low"
    );
}

#[test]
fn fails_when_consecutive_block_difficulty_is_higher_than_bounds() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.header.difficulty =
        first_block.header.difficulty.saturating_add(first_block.header.difficulty / 399);
    second_block.pow = calculate_superblock_effort(second_block.header.difficulty);

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Consecutive Block difficulty is out of bounds"),
        "Expected to fail if the consecutive block difficulty is too high"
    );
}

#[test]
fn fails_when_consecutive_block_timestamp_is_lower() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.header.timestamp = first_block.header.timestamp;

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Block Timestamp is not increasing"),
        "Expected to fail if the consecutive block timestamp is not higher"
    );
}

#[test]
fn fails_when_uncle_number_is_different_from_trunk() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block_uncle = create_uncle(&first_block);
    second_block_uncle.header.number = first_block.header.number + 1;

    let mut second_block = create_child_block(&first_block);
    second_block.uncles = vec![second_block_uncle];

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Uncle's block number does not match trunk block number"),
        "Expected to fail if uncle number is different from trunk number"
    );
}

#[test]
fn fails_when_uncle_parent_is_different_from_trunk() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block_uncle = create_uncle(&first_block);
    second_block_uncle.header.parent = H256::from_low_u64_be(1);

    let mut second_block = create_child_block(&first_block);
    second_block.uncles = vec![second_block_uncle];

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Uncle's parent does not match trunk block's parent"),
        "Expected to fail if uncle parent is different from trunk parent"
    );
}

#[test]
fn fails_when_uncle_difficulty_is_different_from_trunk() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block_uncle = create_uncle(&first_block);
    second_block_uncle.header.difficulty = &first_block.header.difficulty + 1;

    let mut second_block = create_child_block(&first_block);
    second_block.uncles = vec![second_block_uncle];

    let block_list = vec![first_block, second_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Uncle's difficulty does not match trunk block's difficulty"),
        "Expected to fail if uncle has different difficulty from trunk"
    );
}

#[test]
fn block_hash_ignores_base_event_for_now() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let third_block = create_child_block(&second_block);
    let fourth_block = create_child_block(&third_block);
    let block_list = vec![first_block, second_block, third_block, fourth_block];

    let mut args = CheckForkArgsBuilder::new(block_list).required_num_blocks(4).build();
    decorate_required_pegout_events(&mut args);

    let mut lhs = args.block_list[2].header.clone();
    let mut rhs = args.block_list[2].header.clone();

    lhs.base_event = Some([0x11; 32].to_vec());
    lhs.version = 2;
    lhs.hash = lhs.calculate_block_hash().expect("lhs hash");

    rhs.base_event = Some([0x22; 32].to_vec());
    rhs.version = 2;
    rhs.hash = rhs.calculate_block_hash().expect("rhs hash");

    assert_eq!(lhs.hash, rhs.hash);
}

#[test]
fn succeed_if_block_hash_eq_expected_hash() {
    let test_case = serde_json::from_slice::<TestCaseBlockHashValidation>(
        &fs::read("src/tests/block-regtest-min-gas-price-zero.json").unwrap(),
    )
    .unwrap();

    let header = RskBlockHeader::from(&test_case.header);
    let hash = header.calculate_block_hash().unwrap();
    let expected_hash = H256::from_str(&test_case.expected_hash).unwrap();

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

#[test]
fn fails_if_extension_data_is_precompressed_v1() {
    let mut header = create_first_block(DEFAULT_INIT_BLOCK_NUMBER).header;
    let precompressed_extension_data =
        encode_list(vec![alloy_rlp::encode(1_u8), alloy_rlp::encode([0_u8; 32].as_slice())]);
    header.extension_data = precompressed_extension_data;

    let err = header.calculate_block_hash().expect_err(
        "precompressed extension_data should fail because check-fork expects expanded RPC logsBloom",
    );

    assert_eq!(err, "unsupported extension_data format: expected RPC logsBloom (256 bytes)");
}

// TODO add more complex tests, ie: with more than 2 blocks, with more uncles, with more real block data, etc.
fn create_base_block(number: u64, parent: Option<H256>) -> RskBlock {
    let difficulty = U256::from(DEFAULT_DIFFICULTY);
    let timestamp = DEFAULT_TIMESTAMP;
    let mut header = RskBlockHeader {
        number,
        difficulty,
        parent: parent.unwrap_or_default(),
        timestamp,
        ..Default::default()
    };
    header.hash = header.calculate_block_hash().expect("could not calculate block hash");

    RskBlock { uncles: vec![], pow: calculate_superblock_effort(difficulty), header }
}

fn create_first_block(number: u64) -> RskBlock {
    create_base_block(number, None)
}

fn create_child_block(parent: &RskBlock) -> RskBlock {
    let mut child = create_base_block(parent.header.number + 1, Some(parent.header.hash));
    child.header.timestamp = parent.header.timestamp + 100;
    child.header.difficulty = build_valid_consecutive_difficulty(parent);
    child.pow = calculate_superblock_effort(child.header.difficulty);
    // we modified the child, we need to recalculate the hash
    child.header.hash =
        child.header.calculate_block_hash().expect("could not calculate block hash");
    child
}

fn create_uncle(brother: &RskBlock) -> RskBlock {
    let mut uncle = create_base_block(brother.header.number, Some(brother.header.parent));
    uncle.header.timestamp = brother.header.timestamp + 10;
    uncle.header.difficulty = brother.header.difficulty;
    uncle.pow = calculate_superblock_effort(uncle.header.difficulty);
    // we modified the uncle, we need to recalculate the hash
    uncle.header.hash =
        uncle.header.calculate_block_hash().expect("could not calculate block hash");
    uncle
}

fn build_valid_consecutive_difficulty(first_block: &RskBlock) -> U256 {
    first_block.header.difficulty + first_block.header.difficulty / 400 // limit threshold
}

fn calculate_superblock_effort(difficulty: U256) -> H256 {
    H256::from(
        U256::MAX
            .checked_div(difficulty)
            .and_then(|n| n.checked_div(U256::from(SUPERBLOCK_TIMES_DIFFICULTY)))
            .expect("0 division on calculate_superblock_effort")
            .to_big_endian(),
    )
}

fn calculate_effort_from_pow(pow: H256) -> U256 {
    let pow_dec = U256::from_big_endian(pow.as_bytes());
    U256::MAX.checked_div(pow_dec).expect("0 division on calculate_effort_from_pow")
}

fn decorate_required_pegout_events(args: &mut CheckForkArgs) {
    let pegout_id = compute_pegout_id(args);
    for index in 2..args.block_list.len() {
        args.block_list[index].header.version = 2;
        args.block_list[index].header.base_event = Some(pegout_id.as_bytes().to_vec());
    }
}

fn assert_minichain_hashes_are_valid_from_fixture(path: &str) {
    let test_cases =
        serde_json::from_slice::<TestCaseMiniChainHashValidation>(&fs::read(path).unwrap())
            .unwrap();

    for (i, block) in test_cases.chain.iter().enumerate() {
        let header = RskBlockHeader::from(&block.header);
        let calculated_hash = header.calculate_block_hash().unwrap();
        let expected_hash = H256::from_str(&block.expected_hash).unwrap();

        assert_eq!(
            calculated_hash, header.hash,
            "Block hash mismatch at index {i} (height {})",
            header.number
        );
        assert_eq!(
            calculated_hash, expected_hash,
            "Block hash mismatch with expectedHash at index {i} (height {})",
            header.number
        );
    }
}

#[derive(Default)]
struct CheckForkArgsBuilder {
    version: Option<u8>,
    seq_id: Option<u32>,
    rand: Option<u32>,
    stream_id: Option<u32>,
    packet_id: Option<u32>,
    utxo_id: Option<u32>,
    operator_id: Option<[u8; 32]>,
    init_block_time: Option<u64>,
    init_block_number: Option<u64>,
    required_num_blocks: Option<u32>,
    required_effort: Option<U256>,
    block_list: Vec<RskBlock>,
}

impl CheckForkArgsBuilder {
    fn new(block_list: Vec<RskBlock>) -> Self {
        Self { block_list, ..Default::default() }
    }

    fn init_block_time(mut self, init_block_time: u64) -> Self {
        self.init_block_time = Some(init_block_time);
        self
    }

    fn init_block_number(mut self, init_block_number: u64) -> Self {
        self.init_block_number = Some(init_block_number);
        self
    }

    fn required_num_blocks(mut self, required_num_blocks: u32) -> Self {
        self.required_num_blocks = Some(required_num_blocks);
        self
    }

    fn required_effort(mut self, required_effort: U256) -> Self {
        self.required_effort = Some(required_effort);
        self
    }

    fn build(self) -> CheckForkArgs {
        CheckForkArgs {
            version: self.version.unwrap_or(DEFAULT_VERSION),
            seq_id: self.seq_id.unwrap_or(DEFAULT_SEQ_ID),
            rand: self.rand.unwrap_or(DEFAULT_RAND),
            stream_id: self.stream_id.unwrap_or(DEFAULT_STREAM_ID),
            packet_id: self.packet_id.unwrap_or(DEFAULT_PACKET_ID),
            utxo_id: self.utxo_id.unwrap_or(DEFAULT_UTXO_ID),
            operator_id: self.operator_id.unwrap_or(DEFAULT_OPERATOR_ID),
            init_block_time: self.init_block_time.unwrap_or(DEFAULT_TIMESTAMP),
            init_block_number: self.init_block_number.unwrap_or(DEFAULT_INIT_BLOCK_NUMBER),
            required_effort: self.required_effort.unwrap_or(U256::MAX),
            required_num_blocks: self.required_num_blocks.unwrap_or(DEFAULT_REQ_NUMBER_OF_BLOCKS),
            block_list: self.block_list,
        }
    }
}
