# As a transaction dispatcher,
# I want to verify the SPV proof of the user’s funding Bitcoin transaction,
# So that legitimate peg-ins are recorded on-chain

Feature: Register peg-in request

  Background:
    Given the key is created and stored in the keystore
      | keyStorePassword | p09ol.                                             |
      | keyStorePath     | keystore/e5cd9470-cec7-42f4-b1ff-7e723de56793 |
    And the account is funded
    And the contracts are deployed

    # mkdir -p /Users/javier/Projects/UnionBridge/keystore
    # cargo run --bin key-manager new-key -p p09ol. -d /Users/javier/Projects/UnionBridge/keystore
    # cargo run --bin key-manager derive-public-data -p p09ol. -k /Users/javier/Projects/UnionBridge/keystore/e5cd9470-cec7-42f4-b1ff-7e723de56793
    # -> Generated key @ /Users/javier/Projects/UnionBridge/keystore/e5cd9470-cec7-42f4-b1ff-7e723de56793, public '0218a778dc2ca60d1b983c020a32c35902bc7a069db0323469de2ae6491497f970', address '5bdd03ceaf59cad075cb29c67696581d857b9031'
    # anvil
    # cast send --rpc-url http://127.0.0.1:8545 --from 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 5bdd03ceaf59cad075cb29c67696581d857b9031 --value 1ether --unlocked
    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/deploy/deploy-local.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # -> PegManager address: 0xa513E6E4b8f2a923D98304ec87F64353C4D5C853

  Scenario: Happy path
    When the bridge is set up to register the peg-in transaction
    And the dispather is running
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 100000                                                                                                                                           |
      | scriptPubKey0    | 0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd                                                                           |
      | scriptPubKey1    | 0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    Then the response code is "200"
    And the response should contain a valid transaction hash

    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/register-pegin-request.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # KEY_STORE_PASSWORD=p09ol. RUST_BACKTRACE=1 RUST_LOG=debug cargo run --manifest-path /Users/javier/Projects/UnionBridge/union-bridge-client/transaction-dispatcher/Cargo.toml --bin transaction-dispatcher
    # curl -i -X POST http://localhost:3000/register-pegin \    -H "Content-Type: application/json" \    -d '{        "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",        "btc_tx": {            "version": 2,            "outputs": [{                "amount": 100000,                "script_pub_key": "0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd"            },{                "amount": 0,                "script_pub_key": "0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f"            }],            "inputs": [{                "tx_id": "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2",                "v_out": 1694,                "sequence": 4294967293,                "script_sig": ""            }],            "lock_time": 0        },        "merkle_branch_path": "0xFF6B0000",        "merkle_branch_hashes": [            "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"        ]    }'

  Scenario: Register peg-in request already registered
    When the bridge is set up to register the peg-in transaction
    And the dispather is running
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 100000                                                                                                                                           |
      | scriptPubKey0    | 0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd                                                                           |
      | scriptPubKey1    | 0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 100000                                                                                                                                           |
      | scriptPubKey0    | 0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd                                                                           |
      | scriptPubKey1    | 0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    Then the response code is "403"
    And the response contains the error "PeginAlreadyRequested"

    # Do all steps in "Happy path" scenario, but execute twice the POST request

  Scenario: Unsupported denomination
    When the bridge is set up to register the peg-in transaction
    And the dispather is running
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 123456                                                                                                                                           |
      | scriptPubKey0    | 0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd                                                                           |
      | scriptPubKey1    | 0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    Then the response code is "404"
    And the response contains the error "StreamNotFoundByDenomination"

    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/register-pegin-request.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # KEY_STORE_PASSWORD=p09ol. RUST_BACKTRACE=1 RUST_LOG=debug cargo run --manifest-path /Users/javier/Projects/UnionBridge/union-bridge-client/transaction-dispatcher/Cargo.toml --bin transaction-dispatcher
    # curl -i -X POST http://localhost:3000/register-pegin \    -H "Content-Type: application/json" \    -d '{        "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",        "btc_tx": {            "version": 2,            "outputs": [{                "amount": 123456,                "script_pub_key": "0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd"            },{                "amount": 0,                "script_pub_key": "0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f"            }],            "inputs": [{                "tx_id": "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2",                "v_out": 1694,                "sequence": 4294967293,                "script_sig": ""            }],            "lock_time": 0        },        "merkle_branch_path": "0xFF6B0000",        "merkle_branch_hashes": [            "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"        ]    }'

  Scenario: Wrong scriptPubKey - OUTPUT[0]
    When the bridge is set up to register the peg-in transaction
    And the dispather is running
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 100000                                                                                                                                           |
      | scriptPubKey0    | 0x5120228f281f297fd02cd363b9c93f742ba2976c1ec5a6083d9f754cb61e505356c3                                                                           |
      | scriptPubKey1    | 0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    Then the response code is "400"
    And the response contains the error "IncorrectOutputScript"

    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/register-pegin-request.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # KEY_STORE_PASSWORD=p09ol. RUST_BACKTRACE=1 RUST_LOG=debug cargo run --manifest-path /Users/javier/Projects/UnionBridge/union-bridge-client/transaction-dispatcher/Cargo.toml --bin transaction-dispatcher
    # curl -i -X POST http://localhost:3000/register-pegin \    -H "Content-Type: application/json" \    -d '{        "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",        "btc_tx": {            "version": 2,            "outputs": [{                "amount": 100000,                "script_pub_key": "0x5120228f281f297fd02cd363b9c93f742ba2976c1ec5a6083d9f754cb61e505356c3"            },{                "amount": 0,                "script_pub_key": "0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f"            }],            "inputs": [{                "tx_id": "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2",                "v_out": 1694,                "sequence": 4294967293,                "script_sig": ""            }],            "lock_time": 0        },        "merkle_branch_path": "0xFF6B0000",        "merkle_branch_hashes": [            "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"        ]    }'

  Scenario: Wrong OP_RETURN (scriptPubKey - OUTPUT[1])
    When the bridge is set up to register the peg-in transaction
    And the dispather is running
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 100000                                                                                                                                           |
      | scriptPubKey0    | 0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd                                                                           |
      | scriptPubKey1    | 0x6b452534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    Then the response code is "400"
    And the response contains the error "Failed to parse RegisterPegInInput"

    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/register-pegin-request.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # KEY_STORE_PASSWORD=p09ol. RUST_BACKTRACE=1 RUST_LOG=debug cargo run --manifest-path /Users/javier/Projects/UnionBridge/union-bridge-client/transaction-dispatcher/Cargo.toml --bin transaction-dispatcher
    # curl -i -X POST http://localhost:3000/register-pegin \    -H "Content-Type: application/json" \    -d '{        "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",        "btc_tx": {            "version": 2,            "outputs": [{                "amount": 100000,                "script_pub_key": "0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd"            },{                "amount": 0,                "script_pub_key": "0x6b452534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f"            }],            "inputs": [{                "tx_id": "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2",                "v_out": 1694,                "sequence": 4294967293,                "script_sig": ""            }],            "lock_time": 0        },        "merkle_branch_path": "0xFF6B0000",        "merkle_branch_hashes": [            "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"        ]    }'

    Scenario: Wrong length to return (scriptPubKey - OUTPUT[1])
    When the bridge is set up to register the peg-in transaction
    And the dispather is running
    And I POST "/register-pegin"
      | blockHash        | 0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af112155555                                                                               |
      | amount           | 100000                                                                                                                                           |
      | scriptPubKey0    | 0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd                                                                           |
      | scriptPubKey1    | 0x6a4452534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f |
      | merkleHashBranch | 0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f                                                                               |
    Then the response code is "400"
    And the response contains the error "IncorrectlyFormedOpReturn"

    # /Users/javier/Projects/UnionBridge/bitvmx-union-bridge-contracts/shell/script/register-pegin-request.sh
    # privKey: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    # KEY_STORE_PASSWORD=p09ol. RUST_BACKTRACE=1 RUST_LOG=debug cargo run --manifest-path /Users/javier/Projects/UnionBridge/union-bridge-client/transaction-dispatcher/Cargo.toml --bin transaction-dispatcher
    # curl -i -X POST http://localhost:3000/register-pegin \    -H "Content-Type: application/json" \    -d '{        "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",        "btc_tx": {            "version": 2,            "outputs": [{                "amount": 100000,                "script_pub_key": "0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd"            },{                "amount": 0,                "script_pub_key": "0x6a4452534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f"            }],            "inputs": [{                "tx_id": "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2",                "v_out": 1694,                "sequence": 4294967293,                "script_sig": ""            }],            "lock_time": 0        },        "merkle_branch_path": "0xFF6B0000",        "merkle_branch_hashes": [            "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"        ]    }'
    