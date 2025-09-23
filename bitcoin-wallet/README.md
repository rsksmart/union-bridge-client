# Wallet CLI

Simple interactive command-line wallet for crafting P2WPKH transactions using the [`bitcoin`](https://crates.io/crates/bitcoin) crate.

## Commands

- `help` – show available commands
- `set_network <bitcoin|testnet|testnet4|signet|regtest>` – select the active network (defaults to `regtest`)
- `import_private_key <wif>` – import a compressed WIF private key for the active network kind
- `generate_address` – create a new P2WPKH key pair and keep the current active address (use `switch_address` to activate it)
- `list_addresses` – list imported wallet addresses (marking the active one)
- `switch_address <bech32>` – switch the active address when multiple private keys are loaded
- `set_rpc <url> [user] [pass]` – configure a Bitcoin Core RPC endpoint used to broadcast transactions (leave user/password blank for cookie or unauthenticated setups)
- `clear_rpc` – remove the configured RPC client
- `start_regtest_client` – launch a regtest `bitcoind` via Docker and configure RPC automatically (requires Docker)
- `register_utxo <txid> <vout> [satoshis]` – register a spendable P2WPKH UTXO; if the amount is omitted the wallet queries the RPC node for the output value
- `list_funds [all]` – show registered UTXOs for the active address or for every address with `all`
- `send_to_pubkey <hex> <satoshis> [count]` – craft one or more spends to a compressed public key (P2WPKH)
- `send_to_address <bech32> <satoshis> [count]` – craft one or more spends to a bech32 P2WPKH address
- `send_test_funds` – on regtest, mine blocks and fund the wallet through the configured RPC endpoint
- `exit` / `quit` – leave the wallet

Each transaction estimates its fee using the configured satoshis-per-byte rate and returns any change to the wallet key as a new registered UTXO (change smaller than the dust limit is added to the miner fee).
When an RPC endpoint is configured the wallet will broadcast each crafted transaction via `sendrawtransaction`.
Multiple compressed WIF keys can be imported at once. Each address keeps its own set of registered UTXOs and you can move between them with `switch_address` without clearing the store.
Registered UTXOs are persisted in a LevelDB database at the path you provide via `--utxo-db`, `WALLET_UTXO_DB`, or the `utxo_db_path` config setting.

## Configuration

At start-up the CLI looks for `wallet.toml` inside a `config/` directory placed next to the executable (falling back to one in the current working directory). Override the location with `--config-dir` / `WALLET_CONFIG_DIR`, or point to a specific file via `--config` / `WALLET_CONFIG`. If the chosen directory or file is missing the CLI aborts.
All configuration values can also be provided via environment variables or command-line flags, with the following precedence:

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
- `WALLET_UTXO_DB` (optional path to LevelDB store)
- `WALLET_CONFIG_DIR`

Use `ub-wallet --help` to see the available command-line overrides.

## Running

```bash
cargo run --release
```

The program is interactive; type commands at the `ub-wallet>` prompt. Use `Ctrl+D` (EOF) or `exit` to leave.
