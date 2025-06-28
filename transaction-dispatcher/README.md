# Transaction Dispatcher

The `transaction-dispatcher` project is a Rust-based application designed to handle and process various Union Bridge transactions.

This project depends on the **Union Bridge Contract Bindings** for interaction with the Smart Contract. These are provided by the `union-contracts` crate (check `Cargo.toml`), which points to [FairgateLabs/bitvmx-union-bridge-contracts](https://github.com/FairgateLabs/bitvmx-union-bridge-contracts). `forge-bind` is used under the hood to generate the bindings.

Proper version handling for the `union-contracts` dependency is crucial to ensure compatibility with the Union Bridge Smart Contracts. Using a fixed tag or version (instead of branch reference) helps maintain stability, avoid unexpected changes, and ensure reproducibility across environments.

## Running Examples

You can interact with the local Union Bridge server using `curl`. Below are examples of submitting peg-in transactions.

### Register Peg-in Example

```bash
curl -i -X POST http://localhost:3000/register-pegin \
  -H "Content-Type: application/json" \
  -d '{
    "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",
    "btc_tx": {
      "version": 2,
      "outputs": [
        {
          "amount": 100000,
          "script_pub_key": "0x5120aa2820738e170f675ca6d8c30a160f9968cd8bb922b0cc0157169a85fafa3ccd"
        },
        {
          "amount": 0,
          "script_pub_key": "0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f"
        }
      ],
      "inputs": [
        {
          "tx_id": "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2",
          "v_out": 1694,
          "sequence": 4294967293,
          "script_sig": ""
        }
      ],
      "lock_time": 0
    },
    "merkle_branch_path": "0xFF6B0000",
    "merkle_branch_hashes": [
      "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"
    ]
  }'
```

### Accept Peg-in Example

```bash
curl -i -X POST http://localhost:3000/accept-pegin \
  -H "Content-Type: application/json" \
  -d '{
    "block_hash": "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",
    "btc_tx": {
      "version": 2,
      "lock_time": 0,
      "inputs": [{
        "tx_id": "0x9a40f6df4226a822b1b952d41d490a3ab91f1826b684c56a05d75be16f0eb088",
        "v_out": 0,
        "sequence": 4294967293,
        "script_sig": "0x"
      }],
      "outputs": [{
        "amount": 99365,
        "script_pub_key": "0x51209687ca13c4fb3fa3ba05c2f9119dda026bfe66f0098dcf9b896a98ecb2e96702"
      }, {
        "amount": 300,
        "script_pub_key": "0x0014298a0fe992f755152a81ee64bdc4cc96d3bb8969"
      }]
    },
    "merkle_branch_path": "0xFF6B0000",
    "merkle_branch_hashes": [
      "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f"
    ]
  }'
```
