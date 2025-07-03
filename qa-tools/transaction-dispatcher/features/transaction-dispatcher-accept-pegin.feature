@transaction-dispatcher @transaction-dispatcher-accept-pegin @STXDISP03 @S7c6f7c7a
Feature: Transaction dispatcher accept pegin

  @TTXD03001
  Scenario: Happy path
    When I POST to "/register-pegin"
    Then the response code should be "200"
    When I POST to "/accept-pegin"
    Then the response code should be "200"
    And the response should contain a valid transaction hash
