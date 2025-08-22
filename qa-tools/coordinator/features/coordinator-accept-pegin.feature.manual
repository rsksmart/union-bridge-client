@coordinator @SCOORDN03
Feature: Coordinator - Accept Peg in process
  As the Union Bridge client coordinator,
  I want to orchestrate the Accept Peg in process
  so that the Bitcoin transaction is properly registered in the Rootstock blockchain

  Background:
    Given the Union Bridge contracts are deployed
    And BitVMX is running
    And the Union Bridge client services are running
    And the account is funded

    # Check README.md in qa-tools (Initial setup) for the setup instructions

    # Terminal window 1:

    # > (cd ../bitvmx-union-bridge-contracts && forge clean); ./run-mocking.sh

    # Watch the logs in the terminal window, search for "deployed at" to set the contract addresses appropriately in the config/qa-coordinator

    # Terminal window 2:
    # > ./run-client.sh --features anvil,zkp --config config/qa

    # Terminal window 3:
    # > cd qa-tools/coordinator
    # > chmod +x commands.sh
    # > source ./commands.sh
    # IMPORTANT!!!
    # > fund

    # Other terminals:
    # > tail -f logs/coordinator.log
    # > tail -f logs/block-indexer.log
    # > tail -f logs/log-indexer.log

  @TCRD03001
  Scenario: Happy path
    When I send the PeginTransactionFound event
      | file               |
      | pegin_request_happy_path.json |
    And I send the SPV proof for pegin request
      | file               | block_hash                                                         | merkle_hash                                                        | merkle_branch_path |
      | pegin_request_happy_path.json | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 | 0x8325a0c5         |
    And I mine enough blocks to confirm the transaction
    And I send the accept pegin event
      | file               | block_hash                                                         | merkle_hash                                                        | merkle_branch_path |
      | pegin_request_happy_path.json | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 | 0x8325a0c5         |
    And I mine enough blocks to confirm the transaction
    Then the pegin accept should be processed successfully

    # > cd qa-tools/coordinator
    # pf --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json
    # pr 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json 0x8325a0c5 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56
    # replace <REPLACE_WITH_peginRequestTxHash_FROM_PR_RESPONSE> in pegin_accept_happy_path.json with the actual pegin request tx hash from the response of the previous command
    # pa 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 --btc-tx-file qa-tools/coordinator/fixtures/pegin_accept_happy_path.json 0x8325a0c5 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56
    # mine

