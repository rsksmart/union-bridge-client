@transaction-dispatcher @transaction-dispatcher-register-pegin @STXDISP02 @S22bfae21
Feature: Transaction dispatcher register pegin

  @TTXD02001
  Scenario: Happy path
    When I POST to "/register-pegin"
    Then the response code should be "200"
    And the response should contain a valid transaction hash

  @TTXD02002
  Scenario: Unsupported denomination
    When I POST to "/register-pegin"
     | amount |
     | 123456 |
    Then the response code should be "404"
    And the response should contain the error "StreamNotFoundByDenomination"

  @TTXD02003
  Scenario: Register peg-in request already registered
    When I POST to "/register-pegin"
     | amount | tx_id | block_hash | merkle_hash | sequence | v_out | script_sig | merkle_branch_path |
     | 100000 | 0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2 | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9 | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f | 4294967293 | 1694 | 0x | 0xFF6B0000 |
    And I POST to "/register-pegin"
     | amount | tx_id | block_hash | merkle_hash | sequence | v_out | script_sig | merkle_branch_path |
     | 100000 | 0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2 | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9 | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f | 4294967293 | 1694 | 0x | 0xFF6B0000 |
    Then the response code should be "403"
    And the response should contain the error "PeginAlreadyRequested"
