# Local Setup

This is the canonical local-setup runbook for this repository. Use it for setup order, shared configuration, the
recommended local workflow, and troubleshooting.

> Developer setup, environment, and local workflow live here. For engineering standards and team conventions
> (commit conventions, hooks, classification) see [`CONTRIBUTING.md`](CONTRIBUTING.md). For Rust coding patterns
> and codebase-specific guidance see [`AGENTS.md`](AGENTS.md).
>
> Not all crates run the same Quality Gate. [`CONTRIBUTING.md` › Scope and classification](CONTRIBUTING.md#scope-and-classification) classifies each crate as production or non-production; a relaxed bar applies to the latter (`cli/*` and `user-api`).

## First-Time Setup

1. Clone this repository.
2. Install the required tooling.
3. Clone the required sibling repositories.
4. Install the git hooks.
5. Export the shared environment variables.
6. Follow the recommended local development path in this document.

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

### Install Git Hooks

Hook installation is automatic on a clean checkout. [cargo-husky](https://github.com/rhysd/cargo-husky) is declared as
a dev-dependency of the `common` crate; the first time it compiles, its `build.rs` copies the hook entrypoints in
[`.cargo-husky/hooks/`](.cargo-husky/hooks/) into `.git/hooks/`. Trigger it with:

```bash
cargo test --no-run
```

`--no-run` skips test execution — we only need the compile step. A plain `cargo build` does **not** trigger it, because
cargo skips dev-dependencies for that command.

**cargo-husky will not overwrite existing files in `.git/hooks/`.** If the directory is already populated — e.g. you
cloned the repo before the cargo-husky migration and still have `rusty-hook` stubs, or any other hook manager was
previously installed — the automatic install silently skips. In that case, use the
[Reinstalling hooks](#reinstalling-hooks) recipe further down.

The hooks shell out to the helper scripts in [`.hooks/`](.hooks/) (CI calls the same scripts) and rely on two
one-time tools:

```bash
# Nightly rustfmt for the formatting hook
rustup component add rustfmt --toolchain nightly

# cargo-sort for Cargo.toml normalisation
cargo install cargo-sort
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
| `KEY_STORE_PASSWORD` | shell or `.envrc`; written into generated `docker-service.env` during setup | local cargo client, setup helpers, Docker operator runtime | required when creating or unlocking member/user keystores |
| `USER_BITCOIN_WIF` | shell or `.envrc` | user flows, wallet helpers, happy-path testing | required for user-facing Bitcoin operations |
| `MEMBER_BITCOIN_WIF` | shell or `.envrc` | `./cli-bitcoin-wallet.sh`, happy-path testing | required for member wallet operations in local happy-path setup and automated flow tests |
| `BITCOIND_URL` | shell or `.envrc` | `./cli-setup-operators.sh` while patching generated BitVMX configs | required before preparing operator artifacts for Docker-backed local flows |
| `SLOTS_PER_PACKAGE` | shell or `.envrc` | coordinator, BitVMX dispute setup, and `./cli-operations.sh` | temporary workaround until sourced from contracts; optional; defaults to `100` |
| `COMMITTEE_MEMBER_COUNT` | shell or `.envrc` | coordinator and `./cli-operations.sh`; passed into `op-funding` calculations | temporary workaround until sourced from contracts; optional; defaults to `4` |
| `COMMITTEE_PROVER_COUNT` | shell or `.envrc` | coordinator and `./cli-operations.sh`; passed into `op-funding` calculations | temporary workaround until sourced from contracts; optional; defaults to `2` |
| `docker-compose.env` | generated under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` | `docker/operator/start-operators.sh` / Docker compose | Docker operator runtime only |
| `docker-service.env` | generated under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` | operator containers | Docker operator runtime only |
| `UB__...` overrides | shell, `.envrc`, CI, or container env | application config loader | use when you need to override TOML config without editing files |
| `docker/local-infra/.env.local` | tracked under `docker/local-infra/` | `start-blockchains.sh` and `start-bitvmx.sh` | local infra Docker scripts only |

Wrapper script note:

- `./cli-infra.sh` and `./cli-run.sh` read environment variables from your current shell, so load `.envrc` with `direnv allow` or export the variables manually before running them.
- `bash tests/run-flows.sh` sources repo-root `.envrc` automatically when `direnv` is not active.

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
2. Generate fresh operator artifacts with `./cli-setup-operators.sh`.
3. Start blockchains and BitVMX in Docker with `./cli-infra.sh`.
4. Run the Union Bridge client locally with `./cli-run.sh`.
5. Use `./cli-operations.sh` for funding, whitelisting, and stream setup.

Use repo-root commands only:

```bash
# Shared local environment
export BASE_STORAGE_PATH="$HOME"
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
export MEMBER_BITCOIN_WIF=<your-member-wif>
export BITCOIND_URL=http://foo:rpcpassword@host.docker.internal:18443
# Temporary until protocol sizing can be sourced from contracts.
export SLOTS_PER_PACKAGE=10
export COMMITTEE_MEMBER_COUNT=4
export COMMITTEE_PROVER_COUNT=2

# Generate fresh operator runtime artifacts
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
- `./cli-setup-operators.sh --help` currently supports `--ops 1-10` and `-y/--yes`, but the documented local infra
  flow remains centered on 4 prepared operators and 4 local BitVMX instances.
- `./cli-infra.sh --help` is the quickest entry point for local blockchains, BitVMX, and background mining.
- for local debugging snapshots, use [backup-local-logs.sh](scripts/backup-local-logs.sh)
  with `local` or `docker` mode to collect Union Client's coordinator and BitVMX client logs into a timestamped directory

### What the Setup Step Produces

`./cli-setup-operators.sh --ops 4` removes the selected existing operator folders after confirmation, then creates
fresh host-side runtime artifacts under
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`, including:

- `union-client/<service>.pem`
- `union-client/<service>.pubkey_hash`
- `keystore/{member,user}`
- `bitvmx/...`
- `docker-compose.env`
- `docker-service.env`

Host-side `keystore/{member,user}` is used by both local cargo mode and Docker operator runs. Docker operator
containers bind-mount the host keystore directory and reuse the existing files; they do not generate replacement keys.
`cli-setup-operators.sh` creates these files via the `key-manager` crate before Docker startup. Setup does not read
secrets back from an old `docker-service.env`; export the intended `KEY_STORE_PASSWORD` before running it, or enter
it when prompted. Use `./cli-setup-operators.sh --ops 4 -y` for non-interactive reset and setup.

### DRP Program Files

The repository ships sample files under `resources/`:

| File | Purpose |
| --- | --- |
| `resources/generic-verifier.elf` | BitVMX union verifier ELF binary |
| `resources/union-verifier.yaml` | BitVMX union verifier program definition |

For the recommended Docker-backed local path, `config/local.toml` already points to `/app/resources/union-verifier.yaml`,
which matches the Docker mounts used by the local BitVMX flow.

For repo-mode BitVMX, `./cli-run.sh --bitvmx-mode repo` injects:

```bash
UB__FLOWS__COMMITTEE__DRP_PROGRAM_DEFINITION=<project_root>/resources/union-verifier.yaml
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
./cli-infra.sh --start-blockchains [--fresh]
./cli-infra.sh --start-bitvmx [--fresh]
./cli-run.sh [--fresh]
bash tests/run-flows.sh
bash tests/run-flows.sh --ops 4
bash tests/run-flows.sh --setup
bash tests/run-flows.sh --committee
bash tests/run-flows.sh --pegin
bash tests/run-flows.sh --pegout
bash tests/run-flows.sh --operator-take
./cli-infra.sh --stop
```

`./cli-infra.sh --start-blockchains` now bootstraps the regtest Bitcoin miner wallet once (101 blocks when needed)
before background mining starts, so the automated happy path only needs to fund the user/member wallet UTXOs.

Notes:

- `./cli-infra.sh --start-blockchains` starts Anvil + bitcoind and background mining.
- Start `./cli-run.sh` for local mode or `docker/operator/start-operators.sh` for docker mode before using the happy-path script.
- Local happy-path runs require `USER_BITCOIN_WIF` and `MEMBER_BITCOIN_WIF`; `tests/run-flows.sh` uses the user wallet for pegin and pegout and the member wallet during setup funding.
- If mining gets stuck, run `./cli-infra.sh --stop-mining` before restarting it.
- `bash tests/run-flows.sh` runs the default `happy` mode.
- `bash tests/run-flows.sh --ops 4` does the same, but shows the optional operator-count override.
- `bash tests/run-flows.sh --setup` runs only the preparation phases: member wallet prep, operator funding, and whitelist.
- `bash tests/run-flows.sh --committee` runs only the committee creation phases: apply-stream and committee completion wait.
- `bash tests/run-flows.sh --pegin` runs only the pegin flow and reuses existing setup and committee state.
- `bash tests/run-flows.sh --pegout` runs only the pegout flow and reuses existing setup and committee state.
- `bash tests/run-flows.sh --operator-take` runs a pegout that forces the operator-take path, writes the selected operator address to `/tmp/FORCE_ADVANCE` in the active runtime, and reuses existing setup and committee state.
- Use `./cli-infra.sh --start --fresh` instead when you want the all-in-one stack, including BitVMX, from the outset.

The user flows now require explicit Bitcoin public keys in the request body. For manual testing, the same derivation
used by `tests/run-flows.sh` is:

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

- wrong keystore password: export the intended `KEY_STORE_PASSWORD`, then rerun `./cli-setup-operators.sh --ops 4`
- stale local databases: use `./cli-run.sh --fresh`
- BitVMX or blockchain containers out of sync: use `./cli-infra.sh --start --fresh`
- git hooks not running on commit/push (you can commit/push without `fmt` / `sort` / `clippy` / branch-name /
  commit-message checks firing): see [Reinstalling hooks](#reinstalling-hooks) below.

### Reinstalling hooks

Use this recipe when:

- you cloned the repo before the cargo-husky migration and still have `rusty-hook` stubs in `.git/hooks/`;
- you wiped `.git/hooks/` for any reason;
- you can `git commit` or `git push` without the format / lint / branch-name / commit-message checks running, which
  means cargo-husky's hooks aren't installed.

```bash
# 1. Remove every existing hook file (deletes rusty-hook stubs and anything else
#    blocking cargo-husky from writing the new hooks).
find .git/hooks -type f ! -name '*.sample' -delete

# 2. Force cargo-husky's build.rs to re-run on the next compile. Without this,
#    cargo uses its cached compilation of cargo-husky and the install step is
#    silently skipped.
cargo clean -p cargo-husky

# 3. Trigger a compile of `common`'s dev-deps. cargo-husky's build.rs runs and
#    writes pre-commit / pre-push / commit-msg into .git/hooks/.
cargo test --no-run -p common
```

After this, `.git/hooks/` should contain exactly three files: `pre-commit`, `pre-push`, `commit-msg`. If you previously
installed `rusty-hook` globally, you can also `cargo uninstall rusty-hook` to stop the obsolete warning from appearing
on every commit.

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
