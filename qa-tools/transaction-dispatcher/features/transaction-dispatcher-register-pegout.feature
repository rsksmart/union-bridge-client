@transaction-dispatcher @transaction-dispatcher-register-pegout @STXDISP04 @Sb1156630
Feature: Transaction dispatcher register pegout

  @TTXD04001
  Scenario: Happy path
    When I POST to "/register-pegin"
    Then the response code should be "200"
    When I POST to "/accept-pegin"
    Then the response code should be "200"
    When I POST to "/register-pegout"
    Then the response code should be "200"
    And the response should contain a valid transaction hash

  @TTXD04002
  Scenario: Unsupported denomination
    When I POST to "/register-pegin"
    When I POST to "/accept-pegin"
    When I POST to "/register-pegout"
      | amount_in_wei |
      | 4565465465465 |
    Then the response code should be "404"
    And the response should contain the error "StreamNotFoundByDenomination"

  @TTXD04003
  Scenario: Bad user public key
    When I POST to "/register-pegin"
    When I POST to "/accept-pegin"
    When I POST to "/register-pegout"
      | usr_pub_key |
      | 0xaaa777ccc |
    Then the response code should be "400"
    And the response should contain the error "Failed to parse usr_pub_key"
