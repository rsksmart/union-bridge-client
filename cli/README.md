# Union Bridge CLI Tools

The `cli/` workspace contains the local wrappers and CLI tools used for development, operator operations, and the
Bitcoin wallet helper.

> These tools are intended for development and validation. They are not documented here as production interfaces.

## Related Docs

- [Contributing Guide](../CONTRIBUTING.md): canonical setup order, shared env rules, and the recommended local flow
- [Operator Docker Runtime Guide](../docker/operator/README.md): local Docker operator runtime
- [Wallet CLI Guide](bitcoin-wallet/README.md): wallet-specific commands and behavior

Run the wrapper scripts below from the repository root.

## Workflow Entry Points

- local cargo client + Docker-backed infra: follow the [Contributing Guide](../CONTRIBUTING.md) first, then use the wrapper
  commands below
- local Docker operator runtime: use `--env docker` after following the [Operator Docker Runtime Guide](../docker/operator/README.md)
- remote CLI profile: use `--env <profile>` with a matching `cli/.env.<profile>`

## `cli-run.sh`

Launch one or more Union Bridge clients locally for development and testing.

```bash
./cli-run.sh --help
./cli-run.sh --features anvil
./cli-run.sh --id 1 --features anvil
./cli-run.sh --fresh
./cli-run.sh --bitvmx-mode docker
./cli-run.sh --bitvmx-mode repo
./cli-run.sh --kill
```

## `cli-operations.sh`

Operator and user operations for local, Docker-backed, and remote-profile environments.

User and member wallet addresses are deterministically derived from the staged
keystore and env artifacts under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`.

### Supported Environments

- `local`
- `docker`
- any remote profile name, for example `alphanet`

### Environment Variables

For all environments:

- `UC_ENV`
- `UC_OPERATOR_ID`
- `UC_OPERATOR_ROLE`

For remote CLI access, copy `cli/.env.sample` to `cli/.env.<profile>` and fill in the values there.

### Usage Examples

```bash
./cli-operations.sh --help

# Local Docker funding
./cli-operations.sh operator fund --env docker

# Local operator apply-stream
./cli-operations.sh operator apply-stream --stream 1

# Local whitelist
./cli-operations.sh operator whitelist --contract-address 0x742d35... --env local

# Local user flows
./cli-operations.sh user fund --env local
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --btc-pub-key 0x<32-byte-xonly-pubkey> --env local
./cli-operations.sh user pegout --value 1000000 --usr-pub-key 0x<33-byte-compressed-pubkey> --env local
```

### Remote CLI Notes

Any `--env <name>` other than `local` or `docker` is treated as a remote profile. The CLI looks for `cli/.env.<name>`
and expects these keys there:

- `UC_REMOTE_SSH_USER`
- `UC_REMOTE_HOSTS`
- `UC_REMOTE_USER_API_ENDPOINTS`
- `UC_REMOTE_RPC_URL`

These files are ignored by git.

### Safety Features

- `--execute` is only supported for local environments (`local`, `docker`)
- confirmation prompts remain enabled for remote operations

## Workspace Layout

```text
cli/
├── Cargo.toml
├── Cargo.lock
├── .env.sample         # Template for remote CLI profiles (copy to .env.<profile>)
├── run/                # Local client launcher (cli-run.sh)
│   ├── src/main.rs
│   └── Cargo.toml
├── operations/         # Operations toolkit (cli-operations.sh)
│   ├── src/
│   │   ├── main.rs
│   │   ├── bitcoin_wallet.rs
│   │   ├── rsk_wallet.rs
│   │   ├── committee.rs
│   │   ├── pegin.rs
│   │   ├── pegout.rs
│   │   ├── environments.rs
│   │   ├── constants.rs
│   │   └── utils.rs
│   └── Cargo.toml
└── bitcoin-wallet/     # Interactive Bitcoin wallet (cli-bitcoin-wallet.sh)
    ├── src/
    │   ├── main.rs
    │   ├── lib.rs
    │   ├── cli.rs
    │   ├── config.rs
    │   ├── wallet.rs
    │   ├── utxo_store.rs
    │   ├── pending_tx_store.rs
    │   └── bitcoin/
    └── Cargo.toml
```

The wallet crate currently includes:

```text
cli/bitcoin-wallet/
├── config/
├── src/
│   ├── bitcoin/
│   ├── cli.rs
│   ├── config.rs
│   ├── lib.rs
│   ├── main.rs
│   ├── pending_tx_store.rs
│   ├── utxo_store.rs
│   └── wallet.rs
└── tests/
```


## Docker Integration

`docker-compose.env` and `docker-service.env` are Docker operator runtime artifacts. They are not read by
`./cli-run.sh`, which uses the local cargo-mode keystores staged under `BASE_STORAGE_PATH`.
Docker operator mode reuses the same host `op_N/union-client/keystore/{user,member}` files via bind mounts, so the
keystores created by `cli-setup-operators.sh` serve both local cargo mode and the operator containers.
