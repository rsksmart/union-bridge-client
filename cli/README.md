# Union Bridge CLI Tools

The `cli/` workspace contains the CLI tools for local development, operator operations, and Bitcoin wallet management.

## Related Docs

- [../CONTRIBUTING.md](../CONTRIBUTING.md): contributor setup, shared configuration, and local validation flows
- [../docker/operator/README.md](../docker/operator/README.md): local Docker operator runtime
- [bitcoin-wallet/README.md](bitcoin-wallet/README.md): Bitcoin wallet helper used by some operations

## Workflow Entry Points

- Local cargo workflow: bootstrap with `./cli-setup-operators.sh --ops 4`, then use the commands below.
- Local Docker workflow: use `--env docker` after following [../docker/operator/README.md](../docker/operator/README.md).
- Remote CLI workflow: use a profile name such as `alphanet` with a matching `cli/.env.<profile>` file.

## `cli-run.sh`

Launches one or more Union Bridge clients locally for development and testing.

```bash
./cli-run.sh --help
./cli-run.sh --features anvil
./cli-run.sh --id 1 --features anvil
./cli-run.sh --fresh --features anvil
./cli-run.sh --bitvmx-mode docker
./cli-run.sh --bitvmx-mode repo
./cli-run.sh --logs
./cli-run.sh --kill
```

## `cli-operations.sh`

Handles operator operations and user operations across different environments.

### Supported Environments

- `local`
- `docker`
- any remote profile name, for example `alphanet`

Only `local` and `docker` are documented here as Docker runtime targets.

### Environment Variables

For all environments:

- `UC_ENV`
- `UC_OPERATOR_ID`
- `UC_OPERATOR_ROLE`

For remote CLI access only, copy `cli/.env.sample` to `cli/.env.<profile>` and fill in the values there.
For example, `--env alphanet` loads `cli/.env.alphanet`.

### Usage Examples

```bash
./cli-operations.sh --help

# Local Docker funding
./cli-operations.sh operator fund --env docker

# Local operator apply-stream
./cli-operations.sh operator apply-stream --stream-id 1

# Local whitelist
./cli-operations.sh operator whitelist --contract-address 0x742d35... --env local

# Local user flows
./cli-operations.sh user fund --env local
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --btc-pub-key 0x<32-byte-xonly-pubkey> --env local
./cli-operations.sh user pegout --value 1000000 --usr-pub-key 0x<33-byte-compressed-pubkey> --env local
```

### Remote CLI Notes

Any `--env <name>` other than `local` or `docker` is treated as a remote profile. The CLI looks for
`cli/.env.<name>` and expects these keys there:

- `UC_REMOTE_SSH_USER`
- `UC_REMOTE_HOSTS`
- `UC_REMOTE_USER_API_ENDPOINTS`
- `UC_REMOTE_RPC_URL`

Those files are ignored by git.

### Safety Features

- `--execute` is only supported for local environments (`local`, `docker`)
- confirmation prompts remain enabled for remote operations

The CLI tools are organized in a separate Cargo workspace under `cli/`:

```text
cli/
├── Cargo.toml          # CLI workspace configuration with shared dependencies
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
├── mocks/              # Advance-funds mocking tool (cli-mocking.sh)
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   └── events.rs
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

The CLI workspace is independent from the main Union Bridge workspace, allowing for faster compilation and easier maintenance of CLI-specific code.

## Usage Examples

### Local Development Setup

```bash
# 1. Bootstrap wallets, broker identities, and BitVMX runtime artifacts
./cli-setup-operators.sh --ops 4

# 2. Fund operators (Bitcoin + Rootstock)
./cli-operations.sh operator fund
./cli-operations.sh operator fund --execute

# 3. Run all 4 clients
./cli-run.sh --features anvil

# 4. Apply operators to stream
./cli-operations.sh operator apply-stream --stream-id 1
```

`cli-setup-operators.sh` creates the local keystores consumed by `./cli-run.sh`.
Docker operator mode uses `docker-compose.env` / `docker-service.env` and container keystore paths instead of these cargo-mode keystore files.

## Docker Integration

When using the public `docker/operator` setup, you can use `cli-operations.sh` against local Dockerized operators:

```bash
./cli-operations.sh operator fund --env docker
./cli-operations.sh operator apply-stream --stream-id 1 --env docker
```
