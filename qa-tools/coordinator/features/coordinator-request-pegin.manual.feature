@coordinator @SCOORDN02
Feature: Coordinator - Request Peg in process
  As the Union Bridge client coordinator,
  I want to orchestrate the Request Peg in process
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

  @TCRD02001
  Scenario: Happy path
    When I send the PeginTransactionFound event
      | file               |
      | pegin_request_happy_path.json |
    And I send the SPV proof for pegin request
      | file               | block_hash                                                         | merkle_hash                                                        | merkle_branch_path |
      | pegin_request_happy_path.json | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 | 0x8325a0c5         |
    And I mine enough blocks to confirm the transaction
    Then the pegin request should be processed successfully

    # Execute pf, pr, pa commands where actors mocking is running:
    # pf --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json
    # pr 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json 0x8325a0c5 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56
    # mine 5 blocks
    # you will see errors "Failed to get member communication data" related to committee members not being mocked, but that is fine for this test

  @TCRD02002
  Scenario: Unsupported denomination
    When I send the PeginTransactionFound event
      | file                             |
      | pf_unsupported_denomination.json |
    And I send the SPV proof for pegin request
      | file                             | block_hash                                                         | merkle_hash                                                        | merkle_branch_path |
      | pf_unsupported_denomination.json | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 | 0x8325a0c5         |
    And I mine enough blocks to confirm the transaction
    Then the pegin request should fail with the error message "StreamNotFoundByDenomination"

    # Execute pf, pr, pa commands where actors mocking is running:
    # pf --btc-tx-file qa-tools/coordinator/fixtures/pf_unsupported_denomination.json
    # pr 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 --btc-tx-file qa-tools/coordinator/fixtures/pf_unsupported_denomination.json 0x8325a0c5 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56
    # mine 5 blocks

  @TTXD02003
  Scenario: Register peg-in request already registered
    When I send the PeginTransactionFound event
      | file                             |
      | pegin_request_happy_path.json |
    And I send the SPV proof for pegin request
      | file                             | block_hash                                                         | merkle_hash                                                        | merkle_branch_path |
      | pegin_request_happy_path.json | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 | 0x8325a0c5         |
    And I mine enough blocks to confirm the transaction
    When I send the PeginTransactionFound event
      | file                             |
      | pf_unsupported_denomination.json |
    And I send the SPV proof for pegin request
      | file                             | block_hash                                                         | merkle_hash                                                        | merkle_branch_path |
      | pf_unsupported_denomination.json | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 | 0x8325a0c5         |
    Then the pegin request should fail with the error message "PeginAlreadyRequested"

    # Execute pf, pr, pa commands where actors mocking is running:
    # pf --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json
    # pr 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json 0x8325a0c5 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56
    # mine 5 blocks
    # pf --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json
    # pr 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 --btc-tx-file qa-tools/coordinator/fixtures/pegin_request_happy_path.json 0x8325a0c5 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56
