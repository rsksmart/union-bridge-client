# Wallet CLI

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

Each transaction estimates its fee using the configured satoshis-per-byte rate and returns any change to the wallet key
as a new registered UTXO (change smaller than the dust limit is added to the miner fee).
When an RPC endpoint is configured the wallet will broadcast each crafted transaction via `sendrawtransaction`.
Multiple compressed WIF keys can be imported at once. Each address keeps its own set of registered UTXOs and you can
move between them with `switch_address` without clearing the store.
Registered UTXOs are persisted in a RocksDB database at a path determined as follows:

- If you pass `--utxo-db` or set `WALLET_UTXO_DB` (env), that absolute path is used as-is.
- Otherwise, the path is `BASE_STORAGE_PATH` (env) joined with the relative `utxo_db_path` from the `toml` config file.

## Configuration

At start-up the CLI looks for `wallet.toml` inside a `config/` directory placed next to the executable (falling back to
one in the current working directory). Select which environment config to use via `--env` or `WALLET_ENV` (e.g.,
`regtest`, `testnet`).
If the chosen config file is missing the CLI aborts.
All configuration values can also be provided via environment variables or command-line flags, with the following
precedence:

1. Command-line options (`ub-wallet --network testnet ...`)
2. Environment variables (e.g. `WALLET_RPC_URL`, `WALLET_PRIVATE_KEY`)
3. Values from the TOML config file

Example `config/wallet.toml`:

```toml
network = "regtest"
sats_per_byte = 5
private_key_wif = "L1..."
rpc_url = "http://127.0.0.1:18443"
rpc_user = "user"
rpc_password = "password"
utxo_db_path = "./wallet-utxo-db"

[[utxos]]
txid = "e3d9..."
vout = 0
value_sat = 150000
```

Environment variable shortcuts:

- `WALLET_NETWORK`
- `WALLET_SATS_PER_BYTE`
- `WALLET_PRIVATE_KEY`
- `WALLET_RPC_URL`
- `WALLET_RPC_USER` (optional)
- `WALLET_RPC_PASSWORD` (optional)
- `WALLET_UTXO_DB` (optional path to RocksDB store)

Use `ub-wallet --help` to see the available command-line overrides.

## Running

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

## Important notice

It's important to close the wallet as soon as you are done with it to avoid the wrong wallet shutdown which leaves the
database locked. Check the [Troubleshooting](#troubleshooting) section below.

## Troubleshooting

### Misaligned UTXOs

If the wallet complains about misaligned UTXOs (`bad-txns-inputs-missingorspent` or similar), you can clear the local
UTXO database. At the ub-wallet prompt run: `clear_db`.  **Important**: This operation is Regtest only and must never be
used on mainnet or testnet (it will be prevented by the wallet itself). It clears the local RocksDB for the active
network so you can re-register UTXOs from the node.

### Database access error

If you see an error like _Error: failed to open storage backend: Error creating storage_, it means another instance of
the wallet is already running and has locked the database. Only one wallet instance can run at a time.