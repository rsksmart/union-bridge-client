# As a transaction dispatcher,
# I want to retrieve a Taproot deposit address for an upcoming peg-in,
# So that the user can initiate the bridging process

Feature: Retrieve a temporary peg-in address

  Background:
    Given the key is created and stored in the keystore
      | keyStorePassword | p09ol.                                             |
      | keyStorePath     | keystore/e5cd9470-cec7-42f4-b1ff-7e723de56793 |
    And the contracts are deployed
    And the dispatcher is running

    # mkdir -p /Users/javier/Projects/UnionBridge/keystore
    # cargo run --bin key-manager new-key -p p09ol. -d /Users/javier/Projects/UnionBridge/keystore
    # cargo run --bin key-manager derive-public-data -p p09ol. -k /Users/javier/Projects/UnionBridge/keystore/e5cd9470-cec7-42f4-b1ff-7e723de56793
    # -> Generated key @ /Users/javier/Projects/UnionBridge/keystore/e5cd9470-cec7-42f4-b1ff-7e723de56793, public '0218a778dc2ca60d1b983c020a32c35902bc7a069db0323469de2ae6491497f970', address '5bdd03ceaf59cad075cb29c67696581d857b9031'
    # anvil
    # cast send --rpc-url http://127.0.0.1:8545 --from 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 5bdd03ceaf59cad075cb29c67696581d857b9031 --value 1ether --unlocked
    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/deploy/deploy-local.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # -> PegManager address: 0xa513E6E4b8f2a923D98304ec87F64353C4D5C853
    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/get-temporary-address.sh
    # KEY_STORE_PASSWORD=p09ol. RUST_BACKTRACE=1 RUST_LOG=debug cargo run --manifest-path /Users/javier/Projects/UnionBridge/union-bridge-client/transaction-dispatcher/Cargo.toml --bin transaction-dispatcher

  Scenario: Happy path
    When I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                         |
      | value                     | 100000                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    Then the response code is "200"
    And  the response should contain a valid bitcoin deposit taproot address
    
    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'

  Scenario: Unsupported denomination
    When I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                         |
      | value                     | 123456                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    Then the response code is "404"
    And  the response contains the error "StreamNotFoundByDenomination"

    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 123456,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'

  Scenario: Bad reimbursement key length
    When I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                       |
      | value                     | 100000                                                           |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7 |
    Then the response code is "400"
    And  the response contains the error "Failed to parse btc_reimbursement_pub_key"
    
    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7"    }'

  Scenario: Bad Rootstock destination (not 20 bytes)
    When I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81                             |
      | value                     | 100000                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    Then the response code is "400"
    And  the response contains the error "Failed to parse rootstock_deposit_address"

    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'

  Scenario: Same btcReimbursementPubKey returns the same address (idempotent)
    When I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                         |
      | value                     | 100000                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    And I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                         |
      | value                     | 100000                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    Then the addresses of both responses should be equal
    
    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'
    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'

  Scenario: Different btcReimbursementPubKey returns a different address
    When I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                         |
      | value                     | 100000                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    And I POST "/pegin-address"
      | rootstock_deposit_address | 0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8                         |
      | value                     | 100000                                                             |
      | btc_reimbursement_pub_key | 0xc72a9f6fc8e57f1de000a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1 |
    Then the addresses of both responses should be different
    
    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'
    # curl -i -X POST http://localhost:3000/pegin-address \    -H "Content-Type: application/json" \    -d '{        "rootstock_deposit_address": "0x7Ac5496aee77c1bA1F0854206A26DdA82A81d6d8",        "value": 100000,        "btc_reimbursement_pub_key": "0xc72a9f6fc8e57f1de000a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"    }'
    