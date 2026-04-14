# Contributing

This is the canonical contributor runbook for this repository. Use it for setup order, shared configuration, the
recommended local workflow, and troubleshooting. 

## First-Time Setup

1. Clone this repository.
2. Install the required tooling.
3. Clone the required sibling repositories.
4. Export the shared environment variables.
5. Follow the recommended local development path in this document.

### Clone the Repository

HTTPS and SSH both work. SSH is only needed if your GitHub access model for private dependencies requires it.

```bash
git clone https://github.com/rsksmart/union-bridge-client.git
cd union-bridge-client
```

### Tooling

Required:

- Rust and Cargo
- direnv
- Foundry

Useful but optional:

- Docker, for the recommended local workflow
- `act`, for local GitHub Actions reproduction

```bash
# Rust and Cargo
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# direnv
brew install direnv

# Foundry
curl -L https://foundry.paradigm.xyz | bash
foundryup

# Optional: act, for local GitHub Actions reproduction
brew install act
```

### Required Sibling Repositories

This workspace depends on:

1. Public BitVMX client: [FairgateLabs/rust-bitvmx-client](https://github.com/FairgateLabs/rust-bitvmx-client)
2. Contracts repository: [rsksmart/union-bridge-contracts](https://github.com/rsksmart/union-bridge-contracts)

The public BitVMX repo is needed for repo-mode and for understanding the wider system. The contracts repo is
needed for full workspace builds, Docker image builds, local tests, and CI reproduction.

```bash
git clone https://github.com/FairgateLabs/rust-bitvmx-client.git
git clone https://github.com/rsksmart/union-bridge-contracts.git
```

## Shared Configuration Model

Use `direnv` plus a local `.envrc` for day-to-day development. Start from [.envrc.sample](.envrc.sample), keep only
the values you need locally, and run `direnv allow`.

Configuration ownership is:

- TOML files under `config/` define the base runtime configuration.
- `UB__...` environment variables override TOML values.
- wrapper scripts and Docker docs define how local runtime artifacts are generated and consumed.

### Configuration Matrix

| Variable or file | Where it is set | Who consumes it | When it matters |
| --- | --- | --- | --- |
| `.envrc` | repo root, usually copied from `.envrc.sample` | your shell via `direnv` | recommended place for local developer env vars |
| `BASE_STORAGE_PATH` | shell or `.envrc` | `./cli-run.sh`, `./cli-operations.sh`, `./cli-bitcoin-wallet.sh`, some local scripts | required for local cargo workflows and wallet DB resolution |
| `KEY_STORE_PASSWORD` | shell or `.envrc`; can also be written into generated `docker-service.env` during setup | local cargo client, setup helpers, Docker operator runtime | required when creating or unlocking member/user keystores |
| `USER_BITCOIN_WIF` | shell or `.envrc`; can also be written into generated `docker-service.env` during setup | user flows, wallet helpers, Docker operator runtime, happy-path testing | required for user-facing Bitcoin operations |
| `BITCOIND_URL` | shell or `.envrc` | `./cli-setup-operators.sh` while patching generated BitVMX configs | required before preparing operator artifacts for Docker-backed local flows |
| `docker-compose.env` | generated under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` | `docker/operator/start-operators.sh` / Docker compose | Docker operator runtime only |
| `docker-service.env` | generated under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` | operator containers | Docker operator runtime only |
| `UB__...` overrides | shell, `.envrc`, CI, or container env | application config loader | use when you need to override TOML config without editing files |
| `docker/local-infra/.env.local` | tracked under `docker/local-infra/` | `start-blockchains.sh` and `start-bitvmx.sh` | local infra Docker scripts only |

### `BASE_STORAGE_PATH` Contract

There are two relevant behaviors:

- local cargo flows expect `BASE_STORAGE_PATH` to be exported so the client can resolve storage paths and keystore
  locations consistently
- some setup and Docker scripts fall back to `${BASE_STORAGE_PATH:-$HOME}` when generating or reading staged operator
  artifacts

For a simple local setup, export it explicitly and keep it stable:

```bash
export BASE_STORAGE_PATH="$HOME"
```

### TOML Files and `UB__...` Overrides

Configuration files live under `config/`:

- `config/base.toml`
- `config/{env}.toml`, for example `local.toml`, `docker.toml`, or `ci.toml`

Any value can be overridden with `UB__...` environment variables using double underscores as path separators.

Example:

```toml
[block_broker]
ip = "127.0.0.1"
port = 5672
```

```bash
UB__BLOCK_BROKER__IP=127.0.0.1
UB__BLOCK_BROKER__PORT=5672
```

## Local Development Modes

There are three supported local modes:

| Mode | When to use it | Main docs |
| --- | --- | --- |
| cargo client + Docker infra | default recommended path | this doc + [Local Infra Guide](docker/local-infra/README.md) + [CLI Tools Guide](cli/README.md) |
| cargo client + external or repo BitVMX | advanced debugging or BitVMX development | this doc + [CLI Tools Guide](cli/README.md) |
| all Docker | operator-focused container runtime | this doc + [Operator Docker Runtime Guide](docker/operator/README.md) |

### Mode: Cargo Client + Docker Infra

Use the [Local Development - Recommended Path](#local-development---recommended-path) section below.

### Mode: Cargo Client + External or Repo BitVMX

Use this when BitVMX runs outside the default Docker-backed local stack.

```bash
export BASE_STORAGE_PATH="$HOME"
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
./cli-infra.sh --start-blockchains [--fresh]
# start BitVMX outside this repo
./cli-run.sh --bitvmx-mode repo [--fresh]
```

Use the [CLI Tools Guide](cli/README.md) for follow-up commands.

### Mode: All Docker

Use this mode when the Union Bridge services run from the operator Docker runtime.

```bash
export BITCOIND_URL=http://user:password@localhost:18443
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
./cli-infra.sh --start-blockchains [--fresh]
./cli-setup-operators.sh --ops 4
cd docker/operator
bash start-operators.sh [--fresh] up -d
```

Use the [Operator Docker Runtime Guide](docker/operator/README.md) for runtime flags and troubleshooting.

## Local Development - Recommended Path

This is the canonical local flow for contributors:

1. Export shared env vars.
2. Prepare operator artifacts once with `./cli-setup-operators.sh`.
3. Start blockchains and BitVMX in Docker with `./cli-infra.sh`.
4. Run the Union Bridge client locally with `./cli-run.sh`.
5. Use `./cli-operations.sh` for funding, whitelisting, and stream setup.

Use repo-root commands only:

```bash
# Shared local environment
export BASE_STORAGE_PATH="$HOME"
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
export BITCOIND_URL=http://user:password@localhost:18443

# Generate or refresh operator runtime artifacts
./cli-setup-operators.sh --ops 4

# Start the local blockchain and BitVMX stack
./cli-infra.sh --start --fresh

# Start the Rust client against the local stack
./cli-run.sh --fresh

# Fund operators for the happy path
./cli-operations.sh operator fund

# Whitelist operator committee member addresses
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>

# Apply operators to the stream
./cli-operations.sh operator apply-stream -s 1
```

Notes:

- `./cli-run.sh` defaults to the Docker-backed BitVMX identity mode; use `--bitvmx-mode repo` only for the advanced
  repo-mode path.
- `./cli-setup-operators.sh --help` currently supports `--ops 1-10`, but the documented local infra flow remains
  centered on 4 prepared operators and 4 local BitVMX instances.
- `./cli-infra.sh --help` is the quickest entry point for local blockchains, BitVMX, and background mining.

### What the Setup Step Produces

`./cli-setup-operators.sh --ops 4` creates or refreshes host-side runtime artifacts under
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`, including:

- `union-client/<service>.pem`
- `union-client/<service>.pubkey_hash`
- `keystore/{member,user}`
- `bitvmx/...`
- `docker-compose.env`
- `docker-service.env`

Host-side `keystore/{member,user}` is used by local cargo mode. Docker operator runs use the generated Docker env files
and container keystore paths instead.

### DRP Program Files

The repository ships sample files under `resources/`:

| File | Purpose |
| --- | --- |
| `resources/hello-world.elf` | sample RISC-V ELF binary |
| `resources/hello-world.yaml` | sample BitVMX program definition |

For the recommended Docker-backed local path, `config/local.toml` already points to `/app/resources/hello-world.yaml`,
which matches the Docker mounts used by the local BitVMX flow.

For repo-mode BitVMX, `./cli-run.sh --bitvmx-mode repo` injects:

```bash
UB__BRIDGE__COMMITTEE__DRP_PROGRAM_DEFINITION=<project_root>/resources/hello-world.yaml
```

## Run the Happy-Path

After the local stack is up, you should be able to run the happy path, which involves operator setup and basic user
peg-in and peg-out flows.

```bash
# Fund operators
./cli-operations.sh operator fund

# Whitelist committee member addresses
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>

# Apply operators to the stream
./cli-operations.sh operator apply-stream -s 1

# User flow help
./cli-operations.sh user pegin --help
./cli-operations.sh user pegout --help
```

The `CommitteeRegistry` contract address comes from the deployed contracts configuration used by your environment.

### Automated Happy-Path

For the automated local happy-path flow:

```bash
./cli-infra.sh --start-mine
bash tests/run-happy-path.sh
./cli-infra.sh --stop-mine
```

This assumes the recommended stack is already running and that the relevant Bitcoin WIF env vars are available.

The user flows now require explicit Bitcoin public keys in the request body. For manual testing, the same derivation
used by `tests/run-happy-path.sh` is:

```bash
# 32-byte x-only pubkey for pegin
bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword \
  getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" | \
  jq -r '.descriptor' | \
  sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/0x\1/' | \
  cut -c1-2,5-

# 33-byte compressed pubkey for pegout
bitcoin-cli -regtest -rpcuser=foo -rpcpassword=rpcpassword \
  getdescriptorinfo "wpkh(${USER_BITCOIN_WIF})" | \
  jq -r '.descriptor' | \
  sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/0x\1/'
```

## Troubleshooting Index

Use the narrow docs for localized problems:

- ports, Docker compose state, local infra volumes: [Local Infra Guide](docker/local-infra/README.md)
- missing `op_N`, `docker-compose.env`, or `docker-service.env`: [Operator Docker Runtime Guide](docker/operator/README.md)
- wrapper flags and CLI examples: [CLI Tools Guide](cli/README.md)
- operator Docker compose variants and `--op` / `--ops`: [Operator Docker Runtime Guide](docker/operator/README.md)
- CI workflows and `act` notes: [Workflow Guide](.github/WORKFLOWS.md)

Common local issues:

- wrong keystore password: rerun `./cli-setup-operators.sh --ops 4` with the intended `KEY_STORE_PASSWORD`
- stale local databases: use `./cli-run.sh --fresh`
- BitVMX or blockchain containers out of sync: use `./cli-infra.sh --start --fresh`

## Team Conventions and Hooks

This repository follows [Conventional Commits](https://www.conventionalcommits.org/en/about/#tooling-for-conventional-commits)
and uses local hooks. See the [Hooks Guide](.hooks/README.md) for hook-specific detail.

Recommended local setup:

```bash
# Install the hook runner used by this repo
cargo install rusty-hook

# Install the git hooks declared in rusty-hook.toml
rusty-hook init

# Install nightly rustfmt for the formatting hook
rustup component add rustfmt --toolchain nightly

# Install cargo-sort for Cargo.toml normalization
cargo install cargo-sort
```

## Advanced Appendix

### cargo Client + repo BitVMX

Use this only when you intentionally want Union Client and BitVMX both running from Rust workspaces.

The manual part is aligning each BitVMX `config/op_N.yaml` with the matching Union Bridge coordinator pubkey hash under
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/coordinator.pubkey_hash`.

For example:

```bash
cat "${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/union-client/coordinator.pubkey_hash"
```

Then set that value in the matching BitVMX config:

```yaml
components:
  l2:
    # Operator coordinator pubkey hash used by this local BitVMX config entry
    pubkey_hash: <operator-coordinator-pubkey-hash>
    # L2 operator identifier for this local config entry
    id: 0
```

The Docker-backed setup path patches those generated values for you. This manual alignment only matters when BitVMX is
running directly from its repo configs.

### Committee Sizing

If you change the committee size in the contracts repository, keep it aligned with the number of clients you intend to
run locally. The standard local docs in this repo assume a 4-member setup.

### Force Flags for Local Testing

The coordinator supports force flags only in the `local` environment.

`FORCE_ADVANCE` contains a Rootstock address. The targeted operator skips the signature sub-flow, which lets the
advance-funds timeout happen naturally.

Recommended hot-reloadable form:

```bash
echo "0xOPERATOR_ADDRESS" > /tmp/FORCE_ADVANCE
rm /tmp/FORCE_ADVANCE
```

Startup-only alternative:

```bash
FORCE_ADVANCE=0xOPERATOR_ADDRESS ./cli-run.sh
```

### Manual Wallet and Key Setup

`./cli-setup-operators.sh` is the standard way to prepare local keystores. If you need the crate-level commands
directly, use:

- [Key Manager Guide](key-manager/README.md)
- [Wallet CLI Guide](cli/bitcoin-wallet/README.md)

### CheckFork and ZKP Reference Flow

The CheckFork tester and the Stark/Snark flow are preserved as reference material only. They are not part of the core
local contributor happy path.

Start from the [CheckFork Guide](check-fork/README.md) and the `check-fork/tester/` tooling when you specifically need
that integration work.
