# Union Bridge CLI Tools

The `cli/` workspace contains the local wrappers and CLI tools used for development, operator operations, and the
Bitcoin wallet helper.

> These tools are intended for development and validation. They are not documented here as production interfaces.

## Related Docs

- [Local Setup Guide](../LOCAL_SETUP.md): canonical setup order, shared env rules, and the recommended local flow
- [Operator Docker Runtime Guide](../docker/operator/README.md): local Docker operator runtime
- [Wallet CLI Guide](bitcoin-wallet/README.md): wallet-specific commands and behavior

Run the wrapper scripts below from the repository root.

## Workflow Entry Points

- local cargo client + Docker-backed infra: follow the [Local Setup Guide](../LOCAL_SETUP.md) first, then use the wrapper
  commands below
- local Docker operator runtime: use `--env docker-anvil` (or `--env docker-rskj`) after following the [Operator Docker Runtime Guide](../docker/operator/README.md)
- remote CLI profile: use `--env <profile>` with a matching `cli/.env.<profile>`

For the automated happy-path script:

- `bash scripts/test-flows.sh --setup` prepares member/operator state only
- `bash scripts/test-flows.sh --committee` applies operators to the stream and waits for committee completion
- `bash scripts/test-flows.sh --pegin`, `--pegout`, and `--operator-take` reuse the prepared state
- the script does not start `scripts/run-infra.sh`, `scripts/run-clients.sh`, or `docker/operator/start-operators.sh` for you

## `scripts/run-clients.sh`

Launch one or more Union Bridge clients locally for development and testing.

```bash
./scripts/run-clients.sh --help
./scripts/run-clients.sh                       # default: local-anvil (--features anvil, --config local-anvil)
./scripts/run-clients.sh --id 1
./scripts/run-clients.sh --rskj                # local-rskj: --config local-rskj, no anvil feature
./scripts/run-clients.sh --fresh
./scripts/run-clients.sh --bitvmx-mode docker
./scripts/run-clients.sh --bitvmx-mode repo
./scripts/run-clients.sh --kill
```

## `scripts/operations.sh`

Operator and user operations for local, Docker-backed, and remote-profile environments.

Address sources now differ by flow:

- member Bitcoin and member RSK addresses come from `/member/funding-info`, exposed by `user-api`
  and backed by coordinator runtime state
- user RSK address comes from `/user/rsk-address`, exposed by `user-api`
- user Bitcoin address is derived locally from your shell's `USER_BITCOIN_WIF`

### Supported Environments

- `local-anvil` (default)
- `docker-anvil`
- `local-rskj`
- `docker-rskj`
- any remote profile name, for example `alphanet`

### Environment Variables

For all environments:

- `UC_ENV`
- `UC_OPERATOR_ID`
- `UC_OPERATOR_ROLE`

For remote CLI access, copy `cli/.env.sample` to `cli/.env.<profile>` and fill in the values there.

### Usage Examples

```bash
./scripts/operations.sh --help

# Docker operator funding
./scripts/operations.sh operator fund --env docker-anvil

# Local operator apply-stream
./scripts/operations.sh operator apply-stream --stream 1

# Local whitelist
./scripts/operations.sh operator whitelist --contract-address 0x742d35... --env local-anvil

# Local user flows
./scripts/operations.sh user fund --env local-anvil
./scripts/operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --btc-pub-key 0x<32-byte-xonly-pubkey> --env local-anvil
./scripts/operations.sh user pegout --value 1000000 --usr-pub-key 0x<33-byte-compressed-pubkey> --env local-anvil
```

### Remote CLI Notes

Any `--env <name>` other than `local-anvil`, `docker-anvil`, `local-rskj`, or `docker-rskj` is treated as a remote profile. The CLI looks for `cli/.env.<name>`
and expects these keys there:

- `UC_REMOTE_SSH_USER`
- `UC_REMOTE_HOSTS`
- `UC_REMOTE_USER_API_ENDPOINTS`
- `UC_REMOTE_RPC_URL`

These files are ignored by git.

### Safety Features

- `--execute` is only supported for local environments (`local-anvil`, `docker-anvil`, `local-rskj`, `docker-rskj`)
- confirmation prompts remain enabled for remote operations

## Workspace Layout

```text
cli/
├── Cargo.toml
├── Cargo.lock
├── .env.sample         # Template for remote CLI profiles (copy to .env.<profile>)
├── run/                # Local client launcher (scripts/run-clients.sh)
│   ├── src/main.rs
│   └── Cargo.toml
├── operations/         # Operations toolkit (scripts/operations.sh)
│   ├── src/
│   │   ├── main.rs
│   │   ├── bitcoin_wallet.rs
│   │   ├── rsk_wallet.rs
│   │   ├── committee.rs
│   │   ├── pegin.rs
│   │   ├── pegout.rs
│   │   ├── environments.rs
│   │   ├── member_funding_info.rs
│   │   ├── constants.rs
│   │   └── utils.rs
│   └── Cargo.toml
└── bitcoin-wallet/     # Interactive Bitcoin wallet (scripts/bitcoin-wallet.sh)
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
`./scripts/run-clients.sh`, which uses the local cargo-mode keystores staged under `BASE_STORAGE_PATH`.
Docker operator mode reuses the same host `op_N/union-client/keystore/{user,member}` files via bind mounts, so the
keystores created by `scripts/setup-operators.sh` serve both local cargo mode and the operator containers.
