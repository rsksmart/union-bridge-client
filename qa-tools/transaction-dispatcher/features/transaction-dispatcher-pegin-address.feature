@transaction-dispatcher @transaction-dispatcher-pegin-address
Feature: Transaction dispatcher pegin address

  Scenario: Happy path
    When I POST to "/pegin-address"
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 100000 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    Then the response code should be "200"
    And the response should contain the bitcoin address "bcrt1py28js8ef0lgpe5mrh8yn7apt52tkc8k95cyrm8m4fjmpu5zn2mps7esu9h"

  Scenario: Unsupported denomination
    When I POST to "/pegin-address"
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 123456 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    Then the response code should be "404"
    And the response should contain the error "StreamNotFoundByDenomination"

  Scenario: Bad reimbursement key length
    When I POST to "/pegin-address"
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                        |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 100000 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca |
    Then the response code should be "400"
    And the response should contain the error "Failed to parse btc_reimbursement_pub_key"

  Scenario: Bad Rootstock destination (not 20 bytes)
    When I POST to "/pegin-address"
      | rootstock_deposit_address              | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81 | 100000 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    Then the response code should be "400"
    And the response should contain the error "Failed to parse rootstock_deposit_address"

  Scenario: Same btcReimbursementPubKey returns the same address (idempotent)
    When I POST to "/pegin-address"
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 100000 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    And I POST to "/pegin-address" again
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 100000 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    Then the response code of both responses should be "200"
    Then the addresses of both responses should be equal

  Scenario: Different btcReimbursementPubKey returns a different address
    When I POST to "/pegin-address"
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 100000 | 0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    And I POST to "/pegin-address" again
      | rootstock_deposit_address                  | value  | btc_reimbursement_pub_key                                          |
      | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8 | 100000 | 0x7d235c24420b2f00000c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
    Then the response code of both responses should be "200"
    Then the addresses of both responses should be different