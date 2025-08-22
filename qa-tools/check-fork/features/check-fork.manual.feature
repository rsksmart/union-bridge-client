Feature: Check Fork
    As the check-fork component,
    I want to accept or reject a submitted Rootstock fork proof,
    So that the Union Bridge honours only peg-outs backed by a valid proof of work.

# These tests rely mostly on fixture files located under check-fork/fixtures/
# To execute the test commands, do
# > cd qa-tools

Scenario: Happy path
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "original"
    Then the check_fork component should accept the fork proof
# cargo run --bin check_fork_runner -- -o run -f original

Scenario: First block has a timestamp lower than initial_block_timestamp
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1741065665      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "original"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f original -t 1741065665

Scenario: First block has a block number lower than initial_block_number
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883223           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "original"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f original -i 6883223

Scenario: The cumulative PoW in the blocks is lower than required
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort                                         | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 1157920892373161954235709850086879078532699846656405640 | 100                 |
    When the check_fork component receives a fork proof from the fixture "original"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f original -e 1157920892373161954235709850086879078532699846656405640

Scenario: The number of blocks in the list is lower than the required number of blocks
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 120                 |
    When the check_fork component receives a fork proof from the fixture "original"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f original -r 120

Scenario: Block timestamps not ordered from low to high
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "blockTimestampsNotOrdered"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f blockTimestampsNotOrdered

Scenario: Chain of block parenthood is broken
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "chainOfBlockParenthoodBroken"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f chainOfBlockParenthoodBroken

Scenario: Chain of block numbering is broken
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "chainOfBlockNumberBroken"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f chainOfBlockNumberBroken

Scenario: The difficulty target of one block is above the upper band of the possible difficulty changes
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "difficultyTargetAboveRange"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f difficultyTargetAboveRange

Scenario: The difficulty target of one block is below the upper band of the possible difficulty changes
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "difficultyTargetBelowRange"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f difficultyTargetBelowRange

Scenario: The first block does not contain an event created by the Union Bridge
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "original" without adding a bridge event at first block
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f original -g false

Scenario: The first block and another block both contain an event created by the Union Bridge
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "anotherBlockWithBridgeEvent"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f anotherBlockWithBridgeEvent

Scenario: The first block does not contain an event created by the Union Bridge, another block does
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "anotherBlockWithBridgeEvent"  without adding a bridge event at first block
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f anotherBlockWithBridgeEvent -g false

Scenario: The first block contains an event created by the Union Bridge, but the operatorID of the event does not match the one received as argument
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "firstBlockBridgeEventWrongOperatorID"  without adding a bridge event at first block
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f firstBlockBridgeEventWrongOperatorID -g false

Scenario: The first block contains an event created by the Union Bridge, but the pegOutID of the event does not match the one received as argument
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "firstBlockBridgeEventWrongPegOutID"  without adding a bridge event at first block
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f firstBlockBridgeEventWrongPegOutID -g false

Scenario: The first block contains an event created by the Union Bridge, but the UTXOID of the event does not match the one received as argument
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "firstBlockBridgeEventWrongUTXOID"  without adding a bridge event at first block
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f firstBlockBridgeEventWrongUTXOID -g false

Scenario: Only one block contains an uncle which matches all requirements
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "oneUncleMeetsReqs"
    Then the check_fork component should accept the fork proof
# cargo run --bin check_fork_runner -- -o run -f oneUncleMeetsReqs

Scenario: Only one block contains an uncle which does not match the trunk block number
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "oneUncleWrongBlockNumber"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f oneUncleWrongBlockNumber

Scenario: Only one block contains an uncle which does not match the trunk block parent
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "oneUncleWrongParent"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f oneUncleWrongParent

Scenario: Only one block contains an uncle which does not match the trunk block difficulty
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "oneUncleWrongTrunkDifficulty"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f oneUncleWrongTrunkDifficulty

Scenario: One block contains an uncle which matches all requirements and another block contains an uncle which does not match the trunk block number
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "twoUnclesOneWrongBlockNumber"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f twoUnclesOneWrongBlockNumber

Scenario: One block contains an uncle which matches all requirements and another block contains an uncle which does not match the trunk block parent
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "twoUnclesOneWrongParent"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f twoUnclesOneWrongParent

Scenario: One block contains an uncle which matches all requirements and another block contains an uncle which does not match the trunk block difficulty
    Given the check_fork component is set with parameters:
        | utxo_id | pegout_id | operator_id | init_block_time | init_block_number | required_effort | required_num_blocks |
        | any     | any       | any         | 1701129600      | 6883221           | 123456789       | 100                 |
    When the check_fork component receives a fork proof from the fixture "twoUnclesOneWrongTrunkDifficulty"
    Then the check_fork component should reject the fork proof
# cargo run --bin check_fork_runner -- -o run -f twoUnclesOneWrongTrunkDifficulty
