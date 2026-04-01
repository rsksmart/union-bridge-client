# Wallet CLI

For repository-level setup and workflow context, start with [../../README.md](../../README.md),
[../../CONTRIBUTING.md](../../CONTRIBUTING.md), and [../README.md](../README.md). This README covers the wallet-specific
commands and behavior.

Simple interactive command-line wallet for crafting P2WPKH transactions using the [
`bitcoin`](https://crates.io/crates/bitcoin) crate.

## Commands

- `help` – show available commands
- `exit` / `quit` – leave the wallet
- `import_private_key <wif>` – import a compressed WIF private key for the active network kind
- `generate_address` – create a new P2WPKH key pair and keep the current active address (use `switch_address` to
  activate it)
- `list_addresses` – list imported wallet addresses (marking the active one)
- `switch_address <bech32>` – switch the active address when multiple private keys are loaded
- `register_utxo <txid> <vout> [satoshis]` – register a spendable P2WPKH UTXO; if the amount is omitted the wallet
  queries the RPC node for the output value
- `list_funds [all]` – show registered UTXOs for the active address or for every address with `all`
- `send_to_pubkey <hex_csv> <satoshis> [count]` – create a single transaction paying the specified amount to each
  compressed public key (hex) in the comma-separated list (P2WPKH); repeat the whole transaction by `count` (default 1)
- `send_to_address <addr_csv> <satoshis> [count]` – create a single transaction paying the specified amount to each
  address (P2WPKH bech32 or P2PKH base58) in the comma-separated list; repeat the whole transaction by `count` (default
    1)
- `mine_block` – Regtest only: mine a single block via RPC
- `mine_utxo [satoshis]` – Regtest only: mine and fund the active address with given amount (default 21,000,000 sat),
  then register the UTXO
- `tx_status <txid>` – query the node for a transaction: mined?, confirmations, block hash/height, total outputs
- `clear_db` – Regtest only: clear the UTXO database folder for the current network
- `create_pegin_tx <stream_amount> <packet_number> <pegin_address> <rsk_address>` – create a pegin transaction for the Union Bridge protocol. **Note**: After executing this command, you need to mine one block using `mine_block` to confirm the transaction
- `block_height` – query the current blockchain height from the RPC node

Each transaction estimates its fee using the configured satoshis-per-byte rate and returns any change to the wallet key
as a new registered UTXO (change smaller than the dust limit is added to the miner fee).
When an RPC endpoint is configured the wallet will broadcast each crafted transaction via `sendrawtransaction`.
Multiple compressed WIF keys can be imported at once. Each address keeps its own set of registered UTXOs and you can
move between them with `switch_address` without clearing the store.
Registered UTXOs are persisted in a RocksDB database at a path determined as follows:

- If you pass `--db-path` or set `WALLET_DB_PATH` (env), that absolute path is used as-is.
- Otherwise, the path is `BASE_STORAGE_PATH` (env) joined with the relative `db_path` from the `toml` config file.

## Configuration

At start-up the CLI loads `config/{env}.toml` (where `{env}` defaults to `regtest`) from a `config/` directory placed
next to the executable (falling back to the current working directory). Select which environment config to use via
`--env` (e.g., `regtest`, `testnet`).
If the chosen config file is missing the CLI aborts.
All configuration values can also be provided via environment variables or command-line flags, with the following
precedence:

1. Command-line flags and environment variables
2. Values from the TOML config file

### Required Environment Variables

Required Bitcoin RPC settings should be provided in either of these ways:
1. In the TOML config file selected by `--env` (for example, `config/regtest.toml` or `config/testnet.toml`), using
   `rpc_url`, `rpc_user`, and `rpc_password`
2. Via command-line flags or environment variables, which override TOML values when present

Environment variable and CLI shortcuts:

| Variable | CLI flag | Description |
|---|---|---|
| `WALLET_RPC_URL` | `--rpc-url` | Bitcoin RPC endpoint (e.g., `http://127.0.0.1:18443`) |
| `WALLET_RPC_USER` | `--rpc-user` | Bitcoin RPC username |
| `WALLET_RPC_PASSWORD` | `--rpc-password` | Bitcoin RPC password |


The wallet exits if any RPC value is missing after config resolution.

Example `config/regtest.toml`:

```toml
network = "regtest"
sats_per_byte = 5
private_key_wif = "L1..."
rpc_url = "http://127.0.0.1:18443"
rpc_user = "user"
rpc_password = "password"
db_path = "./wallet-db"

[[utxos]]
txid = "e3d9..."
vout = 0
value_sat = 150000
```

Environment variable shortcuts: See `cli.rs` or use `ub-wallet --help` to see the available command-line overrides.

## Running

### Interactive Mode

Within the `bitcoin-wallet/` directory, build and run the wallet in release mode:

Runs by default in `regtest` mode:

```bash
cargo run --release
```

Or specify `testnet` (used in Alphanet environment):

```bash
cargo run --release -- --env testnet
```

The program is interactive; type commands at the `ub-wallet>` prompt. Use `Ctrl+D` (EOF) or `exit` to leave.

### Command Mode (Programmatic Access)

⚠️ **Command mode is restricted to regtest only for safety.** For testnet/mainnet operations, use interactive mode.

The same script supports command mode for programmatic/scripted execution by passing the command as arguments:

```bash
# mine a block
./cli-bitcoin-wallet.sh user mine_block

# mine a utxo with custom amount
./cli-bitcoin-wallet.sh user mine_utxo 50000000

# send to address
./cli-bitcoin-wallet.sh user send_to_address bcrt1q... 10000

# create pegin transaction
./cli-bitcoin-wallet.sh user create_pegin_tx 50000000 1 bcrt1p... 0x1234...

# list funds
./cli-bitcoin-wallet.sh user list_funds
```

You can also use `cargo run` directly:

```bash
cargo run --release --bin ub-wallet -- --mode user mine_block
```

## Important Notice

### Database Locking

The wallet uses RocksDB which allows only **one instance at a time** per database. This means:

- ❌ You **cannot** have an interactive session open while running commands
- ❌ You **cannot** run multiple commands simultaneously
- ✅ You **can** run multiple commands sequentially (they open/close the database)
- ✅ You **can** switch between interactive and command mode (but not simultaneously)

**Example of what doesn't work:**
```bash
# Terminal 1: Opens interactive mode
./cli-bitcoin-wallet.sh user
user@regtest> # keeps database locked while prompt is open

# Terminal 2: This will fail with a clear error message
./cli-bitcoin-wallet.sh user mine_block  # ERROR: Database is locked
```

**Example of what works:**
```bash
# Sequential commands work fine
./cli-bitcoin-wallet.sh user mine_block    # opens, executes, closes
./cli-bitcoin-wallet.sh user list_funds    # opens, executes, closes
./cli-bitcoin-wallet.sh user mine_block    # opens, executes, closes
```

If you see a "Database is locked" error, the wallet will provide helpful suggestions on how to resolve it.

It's important to close the wallet (type `exit` or press `Ctrl+D`) as soon as you are done with interactive mode to avoid leaving the database locked. Check the [Troubleshooting](#troubleshooting) section below.

## Troubleshooting

### Misaligned UTXOs

If the wallet complains about misaligned UTXOs (`bad-txns-inputs-missingorspent` or similar), you can clear the local
UTXO database. At the ub-wallet prompt run: `clear_db`.  **Important**: This operation is Regtest only and must never be
used on mainnet or testnet (it will be prevented by the wallet itself). It clears the local RocksDB for the active
network so you can re-register UTXOs from the node.

### Database locked error

If you see an error like:
```
Error: Database is locked by another wallet instance.
```

This means another wallet instance is currently using the database. The error message will provide:
- The reason (interactive mode open, command running, or crashed process)
- Solutions to resolve the issue
- The database path being locked

**Common solutions:**
1. **Close interactive sessions**: If you have the wallet open in interactive mode, type `exit` or press `Ctrl+D`
2. **Wait for commands to complete**: If a command is running in another terminal, wait for it to finish
3. **Kill zombie processes**: If a previous instance crashed, find and kill it:
   ```bash
   ps aux | grep ub-wallet
   kill <pid>
   ```

Only one wallet instance can access the database at a time.
