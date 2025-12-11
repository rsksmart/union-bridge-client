use std::fs;

use crate::{
    Block, BridgeEvent, CheckForkArgs, SUPERBLOCK_TIMES_DIFFICULTY,
    block_header::TestCaseBlockHashValidation, check_fork,
};
use primitive_types::{H256, U256};

const DEFAULT_DIFFICULTY: u128 = 5_904_436_352_267_687_415_636;
const DEFAULT_TIMESTAMP: u64 = 1000;
const DEFAULT_INIT_BLOCK_NUMBER: u64 = 100;
const DEFAULT_REQ_NUMBER_OF_BLOCKS: u32 = 2;

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
    second_block.number = first_block.number + 2;

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
    second_block.parent = H256::from_low_u64_be(1);

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
fn fails_when_event_not_found_in_first_block() {
    let mut first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    first_block.bridge_event = None;

    let second_block = create_child_block(&first_block);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("First block is missing BridgeEvent"),
        "Expected to fail if event is not found in first block"
    );
}

#[test]
fn fails_when_event_found_in_second_block() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.bridge_event = Some(BridgeEvent {
        utxo_id: "utxo_2".to_string(),
        pegout_id: "pegout_2".to_string(),
        operator_id: "operator_2".to_string(),
    });

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Only the first block should contain a BridgeEvent"),
        "Expected to fail if an event is found in second block"
    );
}

#[test]
fn fails_when_event_has_unexpected_utxo() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let second_block = create_child_block(&first_block);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).event_utxo_id("fake_utxo".to_string()).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("BridgeEvent does not match utxoID"),
        "Expected to fail if the event has a different utxo"
    );
}

#[test]
fn fails_when_event_has_unexpected_pegout_id() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let second_block = create_child_block(&first_block);

    let block_list = vec![first_block, second_block];

    let args =
        CheckForkArgsBuilder::new(block_list).event_pegout_id("fake_pegout".to_string()).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("BridgeEvent does not match pegoutID"),
        "Expected to fail if the event has a different pegout_id"
    );
}

#[test]
fn fails_when_event_has_unexpected_operator() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let second_block = create_child_block(&first_block);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list)
        .event_operator_id("fake_operator".to_string())
        .build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("BridgeEvent does not match operatorID"),
        "Expected to fail if the event has a different operator"
    );
}

#[test]
fn fails_when_consecutive_block_difficulty_is_lower_than_bounds() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.difficulty = first_block.difficulty.saturating_sub(first_block.difficulty / 399);
    second_block.pow = calculate_superblock_effort(second_block.difficulty);

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
    second_block.difficulty = first_block.difficulty.saturating_add(first_block.difficulty / 399);
    second_block.pow = calculate_superblock_effort(second_block.difficulty);

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
    second_block.timestamp = first_block.timestamp;

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
    second_block_uncle.number = first_block.number + 1;

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
    second_block_uncle.parent = H256::from_low_u64_be(1);

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
    second_block_uncle.difficulty = &first_block.difficulty + 1;

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
#[ignore = "TODO: re-enable this check when Superchain is implemented and checked in check_fork (now just logging)"]
fn fails_when_first_block_pow_is_lower_than_required() {
    let mut first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    // make pow lower than required
    first_block.pow = calculate_superblock_effort(first_block.difficulty - 1);

    let second_block = create_child_block(&first_block);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("First block's PoW is less than the required difficulty"),
        "Expected to fail if first block has lower pow than required"
    );
}

#[test]
#[ignore = "TODO: re-enable this check when Superchain is implemented and checked in check_fork (now just logging)"]
fn fails_when_consecutive_block_pow_is_lower_than_required() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);

    // make pow lower than required
    second_block.pow = calculate_superblock_effort(second_block.difficulty - 1);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Consecutive Block's PoW is less than the required difficulty"),
        "Expected to fail if consecutive block has lower pow than required"
    );
}

#[test]
#[ignore = "TODO: re-enable this check when Superchain is implemented and checked in check_fork (now just logging)"]
fn fails_when_uncle_block_pow_is_lower_than_required() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block_uncle = create_uncle(&first_block);
    // make pow lower than required
    second_block_uncle.pow = calculate_superblock_effort(second_block_uncle.difficulty - 1);

    let mut second_block = create_child_block(&first_block);
    second_block.uncles = vec![second_block_uncle];

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = check_fork(&args);
    assert_eq!(
        result,
        Err("Uncle's Block PoW is less than the required difficulty"),
        "Expected to fail if uncle block has lower pow than required"
    );
}

#[test]
fn succeed_if_block_hash_eq_expected_hash() {
    let test_case = serde_json::from_slice::<TestCaseBlockHashValidation>(
        &fs::read("src/tests/block.json").unwrap(),
    )
    .unwrap();

    println!("Rootstock Block: {test_case:?}");

    let hash = test_case.header.calculate_block_hash().unwrap();
    let vec = hash.as_bytes().to_vec();
    let hash_str = hex::encode(vec);

    assert_eq!(
        test_case.expected_hash.get(2..test_case.expected_hash.len()).unwrap(),
        hash_str.as_str()
    );
}

// TODO add more complex tests, ie: with more than 2 blocks, with more uncles, with more real block data, etc.

fn create_base_block(number: u64, bridge_event: bool) -> Block {
    let difficulty = U256::from(DEFAULT_DIFFICULTY);
    Block {
        number,
        hash: H256::from_low_u64_be(number),
        parent: H256::from_low_u64_be(number - 1),
        difficulty,
        timestamp: DEFAULT_TIMESTAMP,
        bridge_event: bridge_event.then(|| BridgeEvent {
            utxo_id: format!("utxo_{number}"),
            pegout_id: format!("pegout_{number}"),
            operator_id: format!("operator_{number}"),
        }),
        uncles: vec![],
        pow: calculate_superblock_effort(U256::from(DEFAULT_DIFFICULTY)),
    }
}

fn create_first_block(number: u64) -> Block {
    create_base_block(number, true)
}

fn create_child_block(parent: &Block) -> Block {
    let mut child = create_base_block(parent.number + 1, false);
    child.timestamp = parent.timestamp + 100;
    child.difficulty = build_valid_consecutive_difficulty(parent);
    child.pow = calculate_superblock_effort(child.difficulty);
    child
}

fn create_uncle(brother: &Block) -> Block {
    let mut uncle = create_base_block(brother.number, false);
    uncle.timestamp = brother.timestamp + 10;
    uncle.difficulty = brother.difficulty;
    uncle.pow = calculate_superblock_effort(uncle.difficulty);
    uncle
}

fn build_valid_consecutive_difficulty(first_block: &Block) -> U256 {
    first_block.difficulty + first_block.difficulty / 400 // limit threshold
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

#[derive(Default)]
struct CheckForkArgsBuilder {
    utxo_id: Option<String>,
    pegout_id: Option<String>,
    operator_id: Option<String>,
    init_block_time: Option<u64>,
    init_block_number: Option<u64>,
    required_num_blocks: Option<u32>,
    required_effort: Option<U256>,
    block_list: Vec<Block>,
}

impl CheckForkArgsBuilder {
    fn new(block_list: Vec<Block>) -> Self {
        CheckForkArgsBuilder { block_list, ..Default::default() }
    }

    fn event_utxo_id(mut self, utxo_id: String) -> Self {
        self.utxo_id = Some(utxo_id);
        self
    }

    fn event_pegout_id(mut self, pegout_id: String) -> Self {
        self.pegout_id = Some(pegout_id);
        self
    }

    fn event_operator_id(mut self, operator_id: String) -> Self {
        self.operator_id = Some(operator_id);
        self
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
            utxo_id: self.utxo_id.unwrap_or_else(|| format!("utxo_{DEFAULT_INIT_BLOCK_NUMBER}")),
            pegout_id: self
                .pegout_id
                .unwrap_or_else(|| format!("pegout_{DEFAULT_INIT_BLOCK_NUMBER}")),
            operator_id: self
                .operator_id
                .unwrap_or_else(|| format!("operator_{DEFAULT_INIT_BLOCK_NUMBER}")),
            init_block_time: self.init_block_time.unwrap_or(DEFAULT_TIMESTAMP),
            init_block_number: self.init_block_number.unwrap_or(DEFAULT_INIT_BLOCK_NUMBER),
            required_effort: self.required_effort.unwrap_or(U256::MAX),
            required_num_blocks: self.required_num_blocks.unwrap_or(DEFAULT_REQ_NUMBER_OF_BLOCKS),
            block_list: self.block_list,
        }
    }
}
