# Actors Mocking CLI

Mock Union Bridge Contracts and BitVMX Client interactions.

## Usage

### Register Pegout Command

The `register-pegout` (or `rpo`) command can be used in two ways:

#### Option 1: Individual Parameters (Original Method)

```bash
register-pegout <block_hash> --btc-tx-file <path_to_btc_tx.json> <merkle_branch_path> <merkle_branch_hashes>
```

Example:
```bash
register-pegout 000000000000000000031234567890abcdef1234567890abcdef1234567890 \
  --btc-tx-file tests/resources/btc_tx_for_register_pegout.json \
  0x1234 \
  1234567890abcdef,abcdef1234567890,567890abcdef1234
```

#### Option 2: JSON File (New Method)

```bash
register-pegout --json-file <path_to_params.json>
```

Example:
```bash
register-pegout --json-file tests/resources/register_pegout_params_example.json
```

### JSON File Format

When using the `--json-file` option, the JSON file should contain all parameters in the following structure:

```json
{
  "block_hash": "000000000000000000031234567890abcdef1234567890abcdef1234567890",
  "btc_tx": {
    "version": 2,
    "lock_time": 0,
    "input": [...],
    "output": [...]
  },
  "merkle_branch_path": "0x1234",
  "merkle_branch_hashes": [
    "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    "567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234"
  ]
}
```

Note: 
- The `btc_tx` field contains the complete Bitcoin transaction object
- The `merkle_branch_hashes` field is an array of strings (not comma-separated)
- When using `--json-file`, you cannot use individual parameters (they are mutually exclusive)

## Other Commands

The CLI supports various other commands for mocking Union Bridge contracts and BitVMX events:

- `request-advance-funds` (raf): Invoke request advance funds
- `advance-funds` (kaf): Invoke advance funds with pegout ID
- `pegin-found` (pf): Mock pegin transaction found event
- `pegin-requested` (pr): Mock pegin requested event
- `pegin-accepted` (pa): Mock pegin accepted event
- `pegout-requested` (por): Mock pegout requested event
- `exit`: Exit the CLI

Use `help` or run without arguments to see all available commands and their parameters.