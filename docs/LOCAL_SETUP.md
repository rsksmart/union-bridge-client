# Local Setup

This is the canonical local-setup runbook for this repository. Use it for setup order, shared configuration, the
recommended local workflow, and troubleshooting.

> Developer setup, environment, and local workflow live here. For engineering standards and team conventions
> (commit conventions, hooks, classification) see [`CONTRIBUTING.md`](../CONTRIBUTING.md). For Rust coding patterns
> and codebase-specific guidance see [`AGENTS.md`](../AGENTS.md).
>
> Not all crates run the same Quality Gate. [`CONTRIBUTING.md` › Scope and classification](../CONTRIBUTING.md#scope-and-classification) classifies each crate as production or non-production; a relaxed bar applies to the latter (`cli/*` and `user-api`).

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

Temporary `local-rskj` note: until the native-bridge local-regtest deploy changes are available in
`rsksmart/union-bridge-contracts`, use the fork branch below for the sibling contracts checkout:

```bash
cd ../union-bridge-contracts
git remote add fedejinich https://github.com/fedejinich/union-bridge-contracts.git
git fetch fedejinich
git checkout chore/local-regtest-native-bridge
```

The `local-rskj` and `docker-rskj` flows deploy contracts from this local checkout, so the checked-out contracts branch
controls what gets deployed to Rootstock regtest.

### Install Git Hooks

Hook installation is automatic on a clean checkout. [cargo-husky](https://github.com/rhysd/cargo-husky) is declared as
a dev-dependency of the `common` crate; the first time it compiles, its `build.rs` copies the hook entrypoints in
[`.cargo-husky/hooks/`](../.cargo-husky/hooks/) into `.git/hooks/`. Trigger it with:

```bash
cargo test --no-run
```

`--no-run` skips test execution — we only need the compile step. A plain `cargo build` does **not** trigger it, because
cargo skips dev-dependencies for that command.

**cargo-husky will not overwrite existing files in `.git/hooks/`.** If the directory is already populated — e.g. you
cloned the repo before the cargo-husky migration and still have `rusty-hook` stubs, or any other hook manager was
previously installed — the automatic install silently skips. In that case, use the
[Reinstalling hooks](#reinstalling-hooks) recipe further down.

The hooks shell out to the helper scripts in [`.hooks/`](../.hooks/) (CI calls the same scripts) and rely on two
one-time tools:

```bash
# Nightly rustfmt for the formatting hook
rustup component add rustfmt --toolchain nightly

# cargo-sort for Cargo.toml normalisation
cargo install cargo-sort
```

## Shared Configuration Model

Use `direnv` plus a local `.envrc` for day-to-day development. Start from [.envrc.sample](../.envrc.sample), keep only
the values you need locally, and run `direnv allow`.

Configuration ownership is:

- TOML files under `config/` define the base runtime configuration.
- `UB__...` environment variables override TOML values.
- wrapper scripts and Docker docs define how local runtime artifacts are generated and consumed.

### Configuration Matrix

| Variable or file | Where it is set | Who consumes it | When it matters |
| --- | --- | --- | --- |
| `.envrc` | repo root, usually copied from `.envrc.sample` | your shell via `direnv` | recommended place for local developer env vars |
| `BASE_STORAGE_PATH` | shell or `.envrc` | `./scripts/run-clients.sh`, `./scripts/operations.sh`, `./scripts/bitcoin-wallet.sh`, some local scripts | required for local cargo workflows and wallet DB resolution |
| `KEY_STORE_PASSWORD` | shell or `.envrc`; written into generated `docker-service.env` during setup | local cargo client, setup helpers, Docker operator runtime | required when creating or unlocking member/user keystores |
| `USER_BITCOIN_WIF` | shell or `.envrc` | user flows, wallet helpers, happy-path testing | required for user-facing Bitcoin operations |
| `MEMBER_BITCOIN_WIF` | shell or `.envrc` | `./scripts/bitcoin-wallet.sh`, happy-path testing | required for member wallet operations in local happy-path setup and automated flow tests |
| `BITCOIND_URL` | shell or `.envrc` | `./scripts/setup-operators.sh` while patching generated BitVMX configs | required before preparing operator artifacts for Docker-backed local flows; the value is baked into the BitVMX `op_*.yaml` at setup (not read live), so **re-run `./scripts/setup-operators.sh` and restart BitVMX whenever you change it** |
| `SLOTS_PER_PACKAGE` | shell or `.envrc` | coordinator, BitVMX dispute setup, and `./scripts/operations.sh` | temporary workaround until sourced from contracts; optional; defaults to `100` |
| `COMMITTEE_MEMBER_COUNT` | shell or `.envrc` | coordinator and `./scripts/operations.sh`; passed into `op-funding` calculations | temporary workaround until sourced from contracts; optional; defaults to `4` |
| `COMMITTEE_PROVER_COUNT` | shell or `.envrc` | coordinator and `./scripts/operations.sh`; passed into `op-funding` calculations | temporary workaround until sourced from contracts; optional; defaults to `2` |
| `docker-compose.env` | generated under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` | `docker/operator/start-operators.sh` / Docker compose | Docker operator runtime only |
| `docker-service.env` | generated under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` | operator containers | Docker operator runtime only |
| `UB__...` overrides | shell, `.envrc`, CI, or container env | application config loader | use when you need to override TOML config without editing files |
| `docker/local-infra/.env.anvil`, `.env.rskj` | tracked under `docker/local-infra/` | `start-blockchains.sh` and `start-bitvmx.sh` | local infra Docker scripts only; one file per Rootstock impl |

Wrapper script note:

- `./scripts/run-infra.sh` and `./scripts/run-clients.sh` read environment variables from your current shell, so load `.envrc` with `direnv allow` or export the variables manually before running them.
- `bash scripts/test-flows.sh` sources repo-root `.envrc` automatically when `direnv` is not active.

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
- `config/{env}.toml`, for example `local-anvil.toml`, `docker-anvil.toml`, `local-rskj.toml`, `docker-rskj.toml`, or `ci.toml`

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

### Secrets hygiene

- `.envrc` is your local override file — copy from `.envrc.sample` and fill in real values. **Never commit `.envrc`** (it is gitignored, but verify before staging).
- Never paste `KEY_STORE_PASSWORD`, `USER_BITCOIN_WIF`, `MEMBER_BITCOIN_WIF`, or the contents of broker `.pem` files into issues, PRs, chat, or commit messages. Use placeholders (`<your-password>`) when reproducing commands.
- For code-side handling of secrets (wrapping in `secrecy::SecretString`, redacting `Debug`), see
  [`CONTRIBUTING.md` › Configuration and secrets](../CONTRIBUTING.md#configuration-and-secrets).

## Local Development Modes

There are three supported local modes:

| Mode | When to use it | Main docs |
| --- | --- | --- |
| cargo client + Docker infra | default recommended path | this doc + [Local Infra Guide](../docker/local-infra/README.md) + [CLI Tools Guide](../cli/README.md) |
| cargo client + external or repo BitVMX | advanced debugging or BitVMX development | this doc + [CLI Tools Guide](../cli/README.md) |
| all Docker | operator-focused container runtime | this doc + [Operator Docker Runtime Guide](../docker/operator/README.md) |

### Mode: Cargo Client + Docker Infra

Use the [Local Development - Recommended Path](#local-development---recommended-path) section below.

### Mode: Cargo Client + External or Repo BitVMX

Use this when BitVMX runs outside the default Docker-backed local stack.

```bash
export BASE_STORAGE_PATH="$HOME"
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
./scripts/run-infra.sh --start-blockchains [--fresh]
# start BitVMX outside this repo
./scripts/run-clients.sh --bitvmx-mode repo [--fresh]
```

Use the [CLI Tools Guide](../cli/README.md) for follow-up commands.

### Mode: All Docker

Use this mode when the Union Bridge services run from the operator Docker runtime.

```bash
export BITCOIND_URL=http://user:password@localhost:18443
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
./scripts/run-infra.sh --start-blockchains [--fresh]
./scripts/setup-operators.sh --ops 4
cd docker/operator
bash start-operators.sh [--fresh] up -d
```

Use the [Operator Docker Runtime Guide](../docker/operator/README.md) for runtime flags and troubleshooting.

## Local Development - Recommended Path

This is the canonical local flow for contributors:

1. Export shared env vars.
2. Generate fresh operator artifacts with `./scripts/setup-operators.sh`.
3. Start blockchains and BitVMX in Docker with `./scripts/run-infra.sh`.
4. Run the Union Bridge client locally with `./scripts/run-clients.sh`.
5. Use `./scripts/operations.sh` for funding, whitelisting, and stream setup.

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
./scripts/setup-operators.sh --ops 4

# Start the local blockchain and BitVMX stack
./scripts/run-infra.sh --start-all --fresh

# Start the Rust client against the local stack
./scripts/run-clients.sh --fresh

# Fund operators for the happy path
./scripts/operations.sh operator fund

# Whitelist operator committee member addresses
./scripts/operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>

# Apply operators to the stream
./scripts/operations.sh operator apply-stream -s 1
```

Notes:

- `./scripts/run-clients.sh` defaults to the Docker-backed BitVMX identity mode; use `--bitvmx-mode repo` only for the advanced
  repo-mode path.
- `./scripts/setup-operators.sh --help` currently supports `--ops 1-10` and `-y/--yes`, but the documented local infra
  flow remains centered on 4 prepared operators and 4 local BitVMX instances.
- `./scripts/run-infra.sh --help` is the quickest entry point for local blockchains, BitVMX, and background mining.
- for local debugging snapshots, use [backup-local-logs.sh](../scripts/backup-local-logs.sh)
  with `local` or `docker` mode to collect Union Client's coordinator and BitVMX client logs into a timestamped directory

### What the Setup Step Produces

`./scripts/setup-operators.sh --ops 4` removes the selected existing operator folders after confirmation, then creates
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
`scripts/setup-operators.sh` creates these files via the `key-manager` crate before Docker startup. Setup does not read
secrets back from an old `docker-service.env`; export the intended `KEY_STORE_PASSWORD` before running it, or enter
it when prompted. Use `./scripts/setup-operators.sh --ops 4 -y` for non-interactive reset and setup.

### DRP Program Files

The repository ships sample files under `resources/`:

| File | Purpose |
| --- | --- |
| `resources/generic-verifier.elf` | BitVMX union verifier ELF binary |
| `resources/union-verifier.yaml` | BitVMX union verifier program definition |

For the recommended Docker-backed local path, `config/local-anvil.toml` already points to `/app/resources/union-verifier.yaml`,
which matches the Docker mounts used by the local BitVMX flow.

For repo-mode BitVMX, `./scripts/run-clients.sh --bitvmx-mode repo` injects:

```bash
UB__FLOWS__COMMITTEE__DRP_PROGRAM_DEFINITION=<project_root>/resources/union-verifier.yaml
```

## Run the Happy-Path

After the local stack is up, you should be able to run the happy path, which involves operator setup and basic user
peg-in and peg-out flows.

```bash
# Fund operators
./scripts/operations.sh operator fund

# Whitelist committee member addresses
./scripts/operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>

# Apply operators to the stream
./scripts/operations.sh operator apply-stream -s 1

# User flow help
./scripts/operations.sh user pegin --help
./scripts/operations.sh user pegout --help
```

The `CommitteeRegistry` contract address comes from the deployed contracts configuration used by your environment.

### Automated Happy-Path

For the automated local happy-path flow:

```bash
./scripts/run-infra.sh --start-blockchains [--fresh]
./scripts/run-infra.sh --start-bitvmx [--fresh]
./scripts/run-clients.sh [--fresh]
bash scripts/test-flows.sh
bash scripts/test-flows.sh --ops 4
bash scripts/test-flows.sh --setup
bash scripts/test-flows.sh --committee
bash scripts/test-flows.sh --pegin
bash scripts/test-flows.sh --pegout
bash scripts/test-flows.sh --operator-take
./scripts/run-infra.sh --stop-all
```

`./scripts/run-infra.sh --start-blockchains` now bootstraps the regtest Bitcoin miner wallet once (101 blocks when needed)
before background mining starts, so the automated happy path only needs to fund the user/member wallet UTXOs.

Notes:

- `./scripts/run-infra.sh --start-blockchains` starts Anvil + bitcoind and background mining.
- Start `./scripts/run-clients.sh` for local mode or `docker/operator/start-operators.sh` for docker mode before using the happy-path script.
- Local happy-path runs require `USER_BITCOIN_WIF` and `MEMBER_BITCOIN_WIF`; `scripts/test-flows.sh` uses the user wallet for pegin and pegout and the member wallet during setup funding.
- If mining gets stuck, run `./scripts/run-infra.sh --stop-mining` before restarting it.
- `bash scripts/test-flows.sh` runs the default `happy` mode.
- `bash scripts/test-flows.sh --ops 4` does the same, but shows the optional operator-count override.
- `bash scripts/test-flows.sh --setup` runs only the preparation phases: member wallet prep, operator funding, and whitelist.
- `bash scripts/test-flows.sh --committee` runs only the committee creation phases: apply-stream and committee completion wait.
- `bash scripts/test-flows.sh --pegin` runs only the pegin flow and reuses existing setup and committee state.
- `bash scripts/test-flows.sh --pegout` runs only the pegout flow and reuses existing setup and committee state.
- `bash scripts/test-flows.sh --operator-take` runs a pegout that forces the operator-take path, writes the selected operator address to `/tmp/FORCE_ADVANCE` in the active runtime, and reuses existing setup and committee state.
- Use `./scripts/run-infra.sh --start-all --fresh` instead when you want the all-in-one stack, including BitVMX, from the outset.

The user flows now require explicit Bitcoin public keys in the request body. For manual testing, the same derivation
used by `scripts/test-flows.sh` is:

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

## Minimal verification

When the full Docker stack isn't up, run scoped checks without spinning everything. For exact CI parity (all three workspaces, with the flags pre-push and CI use), call the hook scripts:

```bash
bash .hooks/format-code.sh --check
bash .hooks/check-lints.sh
```

For faster checks scoped to a single crate (no infra needed):

```bash
cargo build -p <crate> --locked
RISC0_SKIP_BUILD=1 cargo clippy -p <crate> --all-targets --all-features --locked -- -D warnings
cargo test -p <crate> --locked
```

Full workspace tests and end-to-end flow tests still require the [Recommended Path](#local-development---recommended-path) or [`scripts/test-flows.sh`](#automated-happy-path).

## Troubleshooting Index

Use the narrow docs for localized problems:

- ports, Docker compose state, local infra volumes: [Local Infra Guide](../docker/local-infra/README.md)
- missing `op_N`, `docker-compose.env`, or `docker-service.env`: [Operator Docker Runtime Guide](../docker/operator/README.md)
- wrapper flags and CLI examples: [CLI Tools Guide](../cli/README.md)
- operator Docker compose variants and `--op` / `--ops`: [Operator Docker Runtime Guide](../docker/operator/README.md)
- CI workflows and `act` notes: [Workflow Guide](../.github/WORKFLOWS.md)

Common local issues:

- wrong keystore password: export the intended `KEY_STORE_PASSWORD`, then rerun `./scripts/setup-operators.sh --ops 4`
- switching chains / changing `BITCOIND_URL` (e.g. bundled ↔ a bring-your-own node): two chain-tied states must be reset, so rerun `./scripts/setup-operators.sh --ops 4` (re-patches the URL into the BitVMX `op_*.yaml`, else `HTTP 401`) **and** start BitVMX with `./scripts/run-infra.sh --start-bitvmx --fresh` (wipes the `db-bitvmx-*` volumes, else "Inconsistent blockchain state"). A plain restart fixes neither.
- stale local databases: use `./scripts/run-clients.sh --fresh`
- BitVMX or blockchain containers out of sync: use `./scripts/run-infra.sh --start-all --fresh`
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
FORCE_ADVANCE=0xOPERATOR_ADDRESS ./scripts/run-clients.sh
```

### Manual Wallet and Key Setup

`./scripts/setup-operators.sh` is the standard way to prepare local keystores. If you need the crate-level commands
directly, use:

- [Key Manager Guide](../crates/key-manager/README.md)
- [Wallet CLI Guide](../cli/bitcoin-wallet/README.md)

### CheckFork and ZKP Reference Flow

The CheckFork tester and the Stark/Snark flow are preserved as reference material only. They are not part of the core
local contributor happy path.

Start from the [CheckFork Guide](../crates/check-fork/README.md) and the `crates/check-fork/tester/` tooling when you specifically need
that integration work.
