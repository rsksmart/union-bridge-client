use std::fs;
use std::str::FromStr;

use check_fork_tester::TesterRskBlockHeader;
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};

use crate::block_header::{RskBlockHeader, encode_list};
use crate::{
    CheckForkArgs, RskBlock, SUPERBLOCK_TIMES_DIFFICULTY, build_check_fork_journal_from_args,
    build_pegout_base_event, build_pegout_id_preimage, check_fork, compute_pegout_id,
};

const DEFAULT_DIFFICULTY: u128 = 5_904_436_352_267_687_415_636;
const DEFAULT_TIMESTAMP: u64 = 1000;
const DEFAULT_INIT_BLOCK_NUMBER: u64 = 100;
const DEFAULT_REQ_NUMBER_OF_BLOCKS: u32 = 2;
const DEFAULT_VERSION: u8 = 1;
const DEFAULT_SEQUENCE_NUMBER: u64 = 1;
const DEFAULT_STREAM_ID: u64 = 1;
const DEFAULT_PACKET_NUMBER: u64 = 1;
const DEFAULT_SLOT_ID: u64 = 4;
const DEFAULT_OPERATOR_TAKE_PUBKEY_PARITY: u8 = 0x02;
const DEFAULT_OPERATOR_TAKE_PUBKEY_XONLY: [u8; 32] = [0x11; 32];
const DEFAULT_BEST_BLOCK_HASH: [u8; 32] = [0x22; 32];

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
fn fails_with_two_blocks_because_a2_requires_a_checked_continuation_event() {
    let mut actual_effort = U256::zero();

    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    actual_effort += calculate_effort_from_pow(first_block.pow);

    let second_block = create_child_block(&first_block);
    actual_effort += calculate_effort_from_pow(second_block.pow);

    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).required_effort(actual_effort).build();
    let result = run_check_fork(&args);

    assert_eq!(
        result,
        Err("Check-fork A2 requires at least one continuation block with the PegOutID base event"),
        "Expected to fail because A2 needs a checked continuation event"
    );
}

#[test]
fn fails_with_two_blocks_and_one_uncle_because_a2_requires_a_checked_continuation_event() {
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
    let result = run_check_fork(&args);

    assert_eq!(
        result,
        Err("Check-fork A2 requires at least one continuation block with the PegOutID base event"),
        "Expected to fail because A2 needs a checked continuation event"
    );
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

    let block_list = vec![first_block, second_block, third_block];
    let mut args = CheckForkArgsBuilder::new(block_list)
        .required_num_blocks(3)
        .required_effort(actual_effort)
        .build();
    decorate_required_pegout_events(&mut args);

    let result = run_check_fork(&args);

    assert_eq!(result, Ok(actual_effort), "Expected to succeed for valid continuation blocks");
}

#[test]
fn compute_pegout_id_matches_contract_generate_pegout_id_encoding() {
    let sequence_number_bytes =
        hex::decode("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20").unwrap();
    let operator_take_pubkey_xonly =
        hex_32("393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758");
    let best_block_hash = H256::from_slice(&hex_32(
        "595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778",
    ));
    let args = CheckForkArgs {
        version: 1,
        sequence_number: U256::from_big_endian(&sequence_number_bytes),
        stream_id: 0x2122_2324_2526_2728,
        packet_number: 0x292a_2b2c_2d2e_2f30,
        slot_id: 0x3132_3334_3536_3738,
        operator_take_pubkey_parity: 0x02,
        operator_take_pubkey_xonly,
        best_block_hash,
        init_block_time: DEFAULT_TIMESTAMP,
        init_block_number: DEFAULT_INIT_BLOCK_NUMBER,
        required_effort: U256::MAX,
        required_num_blocks: DEFAULT_REQ_NUMBER_OF_BLOCKS,
        block_list: Vec::new(),
    };
    let expected_preimage = hex::decode(concat!(
        "010102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
        "2122232425262728292a2b2c2d2e2f30313233343536373802",
        "393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758",
        "595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778",
    ))
    .unwrap();
    let expected_pegout_id =
        H256::from_str("b4b063cb0c5ed4c2d1751165c2f759c5da3607421d7c40a2884401d416a6f7a6").unwrap();

    assert_eq!(
        build_pegout_id_preimage(&args).as_slice(),
        expected_preimage.as_slice(),
        "preimage must match OperatorTakeManager._generatePegoutId abi.encodePacked bytes"
    );
    assert_eq!(compute_pegout_id(&args), expected_pegout_id);
}

#[test]
fn journal_layout_is_exactly_76_bytes() {
    for (accepted, accepted_byte) in [(true, 1), (false, 0)] {
        let mut args = CheckForkArgsBuilder::new(vec![
            create_first_block(DEFAULT_INIT_BLOCK_NUMBER),
            create_child_block(&create_first_block(DEFAULT_INIT_BLOCK_NUMBER)),
        ])
        .build();
        args.slot_id = 0x0102_0304_0506_0708;

        let journal = build_check_fork_journal_from_args(&args, accepted).to_bytes();
        let pegout_id = compute_pegout_id(&args);

        assert_eq!(journal.len(), 76);
        assert_eq!(journal[0], args.operator_take_pubkey_parity);
        assert_eq!(&journal[1..33], &args.operator_take_pubkey_xonly);
        assert_eq!(&journal[33..65], pegout_id.as_bytes());
        assert_eq!(&journal[65..73], &args.slot_id.to_be_bytes());
        assert_eq!(journal[73], accepted_byte);
        assert_eq!(&journal[74..76], &u16::from(args.version).to_be_bytes());
    }
}

#[test]
fn fails_when_required_block_number_is_invalid() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let block_list = vec![first_block, second_block];

    let args = CheckForkArgsBuilder::new(block_list).required_num_blocks(0).build();

    let result = run_check_fork(&args);
    assert_eq!(
        result,
        Err("Invalid number of required blocks"),
        "Expected to fail if requested number of blocks are invalid"
    );
}

#[test]
fn fails_when_block_list_has_less_than_three_blocks() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let block_list = vec![first_block];

    let args = CheckForkArgsBuilder::new(block_list).required_num_blocks(1).build();

    let result = run_check_fork(&args);
    assert_eq!(
        result,
        Err("Check-fork A2 requires at least one continuation block with the PegOutID base event"),
        "Expected to fail if block_list has less than three blocks"
    );
}

#[test]
fn fails_when_provided_blocks_are_less_than_required() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
    let second_block = create_child_block(&first_block);
    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];

    let args = CheckForkArgsBuilder::new(block_list).required_num_blocks(4).build();

    let result = run_check_fork(&args);
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
    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];

    let args = CheckForkArgsBuilder::new(block_list).init_block_time(1_000_000).build();

    let result = run_check_fork(&args);
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
    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];

    let args = CheckForkArgsBuilder::new(block_list).init_block_number(1_000_000).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    actual_effort += calculate_effort_from_pow(third_block.pow);

    let block_list = vec![first_block, second_block, third_block];
    let expected_effort = actual_effort + 1;
    let mut args = CheckForkArgsBuilder::new(block_list).required_effort(expected_effort).build();
    decorate_required_pegout_events(&mut args);

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let base_event = build_expected_base_event(&args);
    let first_block = &mut args.block_list[0];
    first_block.header.version = 2;
    first_block.header.base_event = Some(base_event);

    let result = run_check_fork(&args);
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
    block.header.hash =
        block.header.calculate_block_hash().expect("could not calculate block hash");
    args.block_list[3].header.parent = block.header.hash;
    args.block_list[3].header.hash =
        args.block_list[3].header.calculate_block_hash().expect("could not calculate block hash");

    let result = run_check_fork(&args);
    assert_eq!(result, Err("Continuation block is missing the expected PegOutID base event"));
}

#[test]
fn fails_when_base_event_pegout_id_does_not_match_args() {
    for (case, offset) in [("pegout_id_first_byte", 0), ("pegout_id_last_byte", 31)] {
        let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);
        let second_block = create_child_block(&first_block);
        let third_block = create_child_block(&second_block);
        let fourth_block = create_child_block(&third_block);
        let block_list = vec![first_block, second_block, third_block, fourth_block];

        let mut args = CheckForkArgsBuilder::new(block_list).required_num_blocks(4).build();
        decorate_required_pegout_events(&mut args);

        let block = &mut args.block_list[2];
        block.header.base_event.as_mut().expect("base event should exist")[offset] ^= 0x01;
        block.header.hash =
            block.header.calculate_block_hash().expect("could not calculate block hash");
        args.block_list[3].header.parent = block.header.hash;
        args.block_list[3].header.hash = args.block_list[3]
            .header
            .calculate_block_hash()
            .expect("could not calculate block hash");

        let result = run_check_fork(&args);
        assert_eq!(
            result,
            Err("Continuation block is missing the expected PegOutID base event"),
            "Expected to fail when {case} does not match the computed pegout ID"
        );
    }
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

    let result = run_check_fork(&args);
    assert_eq!(result, Err("Block with base event must use header version 2"));
}

#[test]
fn fails_when_consecutive_block_difficulty_is_lower_than_bounds() {
    let first_block = create_first_block(DEFAULT_INIT_BLOCK_NUMBER);

    let mut second_block = create_child_block(&first_block);
    second_block.header.difficulty =
        first_block.header.difficulty.saturating_sub(first_block.header.difficulty / 399);
    second_block.pow = calculate_superblock_effort(second_block.header.difficulty);

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
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

    let third_block = create_child_block(&second_block);
    let block_list = vec![first_block, second_block, third_block];
    let args = CheckForkArgsBuilder::new(block_list).build();

    let result = run_check_fork(&args);
    assert_eq!(
        result,
        Err("Uncle's difficulty does not match trunk block's difficulty"),
        "Expected to fail if uncle has different difficulty from trunk"
    );
}

#[test]
fn block_hash_includes_base_event_for_v2() {
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

    assert_ne!(lhs.hash, rhs.hash);
}

#[test]
fn fails_when_base_event_exceeds_vetiver_limit() {
    let mut header = create_first_block(DEFAULT_INIT_BLOCK_NUMBER).header;
    header.version = 2;
    header.base_event = Some(vec![0xaa; 129]);

    let err = header.calculate_block_hash().expect_err("oversized base_event must fail");

    assert_eq!(err, "base_event exceeds maximum allowed length");
}

#[test]
fn fails_when_header_version_is_not_supported() {
    let mut header = create_first_block(DEFAULT_INIT_BLOCK_NUMBER).header;
    header.version = 3;

    let err = header.calculate_block_hash().expect_err("unsupported header version must fail");

    assert_eq!(err, "unsupported RSK block header version");
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

fn hex_32(value: &str) -> [u8; 32] {
    hex::decode(value).unwrap().try_into().unwrap()
}

fn decorate_required_pegout_events(args: &mut CheckForkArgs) {
    let base_event = build_expected_base_event(args);
    for index in 2..args.block_list.len() {
        args.block_list[index].header.version = 2;
        args.block_list[index].header.base_event = Some(base_event.clone());
        args.block_list[index].header.hash = args.block_list[index]
            .header
            .calculate_block_hash()
            .expect("could not calculate block hash");
        if index + 1 < args.block_list.len() {
            args.block_list[index + 1].header.parent = args.block_list[index].header.hash;
        }
    }
}

fn build_expected_base_event(args: &CheckForkArgs) -> Vec<u8> {
    build_pegout_base_event(args).to_vec()
}

fn run_check_fork(args: &CheckForkArgs) -> Result<U256, &'static str> {
    check_fork(args, compute_pegout_id(args))
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
    sequence_number: Option<U256>,
    stream_id: Option<u64>,
    packet_number: Option<u64>,
    slot_id: Option<u64>,
    operator_take_pubkey_parity: Option<u8>,
    operator_take_pubkey_xonly: Option<[u8; 32]>,
    best_block_hash: Option<H256>,
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
            sequence_number: self
                .sequence_number
                .unwrap_or_else(|| U256::from(DEFAULT_SEQUENCE_NUMBER)),
            stream_id: self.stream_id.unwrap_or(DEFAULT_STREAM_ID),
            packet_number: self.packet_number.unwrap_or(DEFAULT_PACKET_NUMBER),
            slot_id: self.slot_id.unwrap_or(DEFAULT_SLOT_ID),
            operator_take_pubkey_parity: self
                .operator_take_pubkey_parity
                .unwrap_or(DEFAULT_OPERATOR_TAKE_PUBKEY_PARITY),
            operator_take_pubkey_xonly: self
                .operator_take_pubkey_xonly
                .unwrap_or(DEFAULT_OPERATOR_TAKE_PUBKEY_XONLY),
            best_block_hash: self
                .best_block_hash
                .unwrap_or_else(|| H256::from(DEFAULT_BEST_BLOCK_HASH)),
            init_block_time: self.init_block_time.unwrap_or(DEFAULT_TIMESTAMP),
            init_block_number: self.init_block_number.unwrap_or(DEFAULT_INIT_BLOCK_NUMBER),
            required_effort: self.required_effort.unwrap_or(U256::MAX),
            required_num_blocks: self.required_num_blocks.unwrap_or(DEFAULT_REQ_NUMBER_OF_BLOCKS),
            block_list: self.block_list,
        }
    }
}
