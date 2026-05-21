# Wallet CLI

For repository-level setup and the recommended local flow, start with:

- [Repository README](../../README.md)
- [Local Setup Guide](../../LOCAL_SETUP.md)
- [CLI Tools Guide](../README.md)

This README only covers wallet-specific commands and behavior.

Simple interactive command-line wallet for crafting P2WPKH transactions using the
[`bitcoin`](https://crates.io/crates/bitcoin) crate.

## Commands

- `help`: show available commands
- `exit` / `quit`: leave the wallet
- `import_private_key <wif>`: import a compressed WIF private key for the active network kind
- `generate_address`: create a new P2WPKH key pair while keeping the current active address
- `list_addresses`: list imported addresses and mark the active one
- `switch_address <bech32>`: change the active address when multiple keys are loaded
- `register_utxo <txid> <vout> [satoshis]`: register a spendable P2WPKH UTXO; if `satoshis` is omitted the wallet asks
  the RPC node for the output value
- `list_funds [all]`: show registered UTXOs for the active address or for all loaded addresses
- `send_to_pubkey <hex_csv> <satoshis> [count]`: create a single tx paying each compressed public key in the
  comma-separated list; repeat the whole transaction by `count`
- `send_to_address <addr_csv> <satoshis> [count]`: create a single tx paying each address in the comma-separated list;
  repeat the whole transaction by `count`
- `mine_block`: regtest only; mine a single block via RPC
- `mine_utxo [satoshis]`: regtest only; fund the active address, then register that UTXO locally
- `tx_status <txid>`: query mined status, confirmations, block info, and output totals
- `clear_db`: regtest only; clear the local wallet RocksDB for the current network
- `create_pegin_tx <stream_amount> <packet_number> <pegin_address> <rsk_address>`: create a pegin transaction for the
  Union Bridge flow. After using it, mine one block with `mine_block` to confirm the transaction
- `block_height`: query the current blockchain height

Each transaction estimates its fee using the configured satoshis-per-byte rate and returns change to the wallet as a
new registered UTXO when applicable. When RPC is configured the wallet also broadcasts the crafted transaction with
`sendrawtransaction`.

## Configuration

At startup the CLI loads `config/{env}.toml` (default `regtest`) from a `config/` directory placed next to the
executable, falling back to the current working directory. You can also override values with CLI flags or environment
variables.

Configuration precedence is:

1. command-line flags and their mapped environment variables
2. values from the selected TOML file

Required RPC inputs can come from either source above. The wallet exits if `rpc_url`, `rpc_user`, or `rpc_password`
are still missing after resolution.

| Variable | CLI flag | Description |
| --- | --- | --- |
| `WALLET_RPC_URL` | `--rpc-url` | Bitcoin RPC endpoint |
| `WALLET_RPC_USER` | `--rpc-user` | Bitcoin RPC username |
| `WALLET_RPC_PASSWORD` | `--rpc-password` | Bitcoin RPC password |

Database path resolution is:

1. `--db-path` or `WALLET_DB_PATH`: use that path as-is
2. otherwise, read `db_path` from the selected TOML and resolve it under `BASE_STORAGE_PATH`

When you rely on TOML `db_path`, `BASE_STORAGE_PATH` must be set.

Example `config/regtest.toml`:

```toml
network = "regtest"
sats_per_byte = 5
rpc_url = "http://127.0.0.1:18443/"
rpc_user = "foo"
rpc_password = "rpcpassword"
db_path = ".union_bridge/bitcoin-wallet"
```

## Running

### Interactive Mode

Run these commands from `cli/bitcoin-wallet/`:

```bash
cargo run --release
cargo run --release -- --env testnet
```

The program is interactive; type commands at the `ub-wallet>` prompt and leave with `Ctrl+D`, `exit`, or `quit`.

### Command Mode

Command mode is regtest-only and is intended for scripted local usage.

```bash
# From the repository root, use the wrapper
./cli-bitcoin-wallet.sh user mine_block
./cli-bitcoin-wallet.sh user mine_utxo 50000000
./cli-bitcoin-wallet.sh user send_to_address bcrt1q... 10000
./cli-bitcoin-wallet.sh user create_pegin_tx 50000000 1 bcrt1p... 0x1234...
./cli-bitcoin-wallet.sh user list_funds

# Or from cli/bitcoin-wallet/, call the binary directly
cargo run --release --bin ub-wallet -- --mode user mine_block
```

## Important Notice

### Database Locking

The wallet uses RocksDB, so only one process can open the same wallet database at a time.

- do not keep an interactive session open while also running scripted commands
- do not run multiple wallet commands simultaneously against the same DB
- sequential commands are fine
- close interactive sessions promptly when you are done

If you see a database-lock error, another wallet process is still using the same DB path.

What does not work:

```bash
# terminal 1
./cli-bitcoin-wallet.sh user

# terminal 2
./cli-bitcoin-wallet.sh user mine_block
```

What works:

```bash
./cli-bitcoin-wallet.sh user mine_block
./cli-bitcoin-wallet.sh user list_funds
./cli-bitcoin-wallet.sh user mine_block
```

## Troubleshooting

### Misaligned UTXOs

If the wallet reports misaligned UTXOs (`bad-txns-inputs-missingorspent` or similar), use `clear_db` from the wallet
prompt. This is regtest-only and is intentionally blocked for testnet or mainnet.

### Database Locked

If you see:

```text
Error: Database is locked by another wallet instance.
```

close any interactive wallet session, wait for running commands to finish, or kill the stale wallet process.
