@coordinator @SCOORDN02
Feature: Coordinator - Request Peg in process
  As the Union Bridge client coordinator,
  I want to orchestrate the Request Peg in process
  so that the Bitcoin transaction is properly registered in the Rootstock blockchain

  @TCRD02001
  Scenario: Happy path
    When bitvmx finds a pegin request
      | file                     | block_hash                                                         | merkle_branch_path | merkle_branch_hashes                                               |  |
      | pegin_request_happy_path | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0x8325a0c5         | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 |  |
    Then the pegin request should be registered in the contract
    When enough confirmations are received
    Then the pegin request should be registed in the coordinator
    When bitvmx accepts a pegin request
      | file                     | block_hash                                                         | merkle_branch_path | merkle_branch_hashes                                               |  |
      | pegin_request_happy_path | 0x5ec8021cc5f6474b479d07f2b5736cba3a894fcd0438099996846379bff35106 | 0x8325a0c5         | 0xcefbc931c000b2b2223e26472dae6ddafea0ace20a442f4ceefc61fe78f13f56 |  |
    Then the pegin accept should be registered in the contract
    When enough confirmations are received
    Then the pegin accept should be registed in the coordinator

