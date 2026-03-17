# Developer Guide

This guide covers setting up and running the Union Bridge Client for development and testing.

## First Time Setup

Before running the Union Bridge Client for the first time:

1. **Clone the repository**
2. **Restore untracked config files** (if you pulled a branch that did the repo cleanup)
3. **Install required tools** (Rust, direnv, Foundry, etc.)
4. **Set up required repositories** (BitVMX Workspace, BitVMX Union Bridge Contracts)
5. **Set up environment variables** using `direnv`
6. **Configure the client** for your environment

### Clone the Repository

```bash
git clone git@github.com:rsksmart/union-bridge-client.git
```

### Restore Untracked Config Files

Environment-specific config files (keys, certs, operator YAMLs, `.env` files) are not tracked by git. After cloning or pulling a branch that removed them, restore them from git history:

```bash
bash scripts/restore-untracked-configs.sh
```

This is idempotent — files that already exist on disk are skipped. The script prefers the current branch’s history so you get the latest structure (e.g. 10 operators, updated op_*.yaml format). You can target specific environments:

```bash
bash scripts/restore-untracked-configs.sh testnet
bash scripts/restore-untracked-configs.sh regtest local
```

### Tooling

#### Required Tools

1. **Rust and Cargo** — The project is built in Rust
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **direnv** — For managing environment variables
   ```bash
   # macOS
   brew install direnv

   # Add to your shell profile (~/.zshrc or ~/.bashrc)
   eval "$(direnv hook zsh)"  # or bash
   ```

3. **Foundry** — Ethereum development toolkit (includes Anvil)
   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

#### Optional Tools

- **Docker** — For containerized deployment (see [docker/README.md](docker/README.md))
- **act** — For running GitHub Actions locally
  ```bash
  brew install act
  ```

### Required Repositories

1. **BitVMX Workspace** — Contains the BitVMX client

   Clone the [BitVMX Workspace](https://github.com/FairgateLabs/rust-bitvmx-workspace) and follow its README. Then run the BitVMX client:

   ```bash
   git clone git@github.com:FairgateLabs/rust-bitvmx-workspace.git
   cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
   ./run_union_example.sh
   ```

   **Note:** Have Docker running before executing the script.

2. **BitVMX Union Bridge Contracts** — Smart contracts for the Union Bridge protocol
   ```bash
   git clone git@github.com:temp-rsk/bitvmx-union-bridge-contracts.git
   ```

   Follow its `README.md` for initial setup.

### Environment Variables Setup

The project uses environment variables for private properties and configuration overrides (see [Configuration](README.md#configuration) in README).

#### Private Properties

The most important environment variables:

- **`KEY_STORE_PASSWORD`** — Password used to create/unlock Rootstock wallets
- **`BASE_STORAGE_PATH`** — Base path where the client stores data (databases, keystore files, etc.)
- **`USER_BITCOIN_WIF`** / **`MEMBER_BITCOIN_WIF`** — Bitcoin WIF keys for peg-in/peg-out and BitVMX operations (see [bitcoin-wallet README](cli/bitcoin-wallet/README.md))

We recommend using `direnv`:

1. Copy [.envrc.sample](.envrc.sample) to `.envrc` in the project root
2. Fill in the values you need (focus on _For local client running_ initially)
3. Run `direnv allow` (and after any change to `.envrc`)

### Multi Client Setup

The multi-client setup is mostly automated via `cli-operations.sh`. The repo supports 1–10 operators (committee size is configurable in the contracts).

#### Creating the Base Directory

Under the directory in `BASE_STORAGE_PATH`:

```bash
mkdir -p .union_bridge
```

The `keystore` subdirectory is created automatically when needed.

#### Generating the Broker Key

The Union Bridge Client uses TLS for broker communication. Generate a broker key:

```bash
openssl genpkey -algorithm RSA -out ${BASE_STORAGE_PATH}/.union_bridge/keystore/broker.key -pkeyopt rsa_keygen_bits:2048
```

This key defines the client’s `pubkey_hash` identity. When using Docker, the BitVMX entrypoint patches `components.l2.pubkey_hash` in the operator YAMLs from the keystore at startup, so local and Docker stay in sync.

#### Configuring the Committee

Edit the contracts (e.g. `bitvmx-union-bridge-contracts/src/CommitteeRegistry.sol`) to match the committee size you want (e.g. 4 or 10):

```solidity
minCommitteeWatchtowers = 2;
minCommitteeOperators = 2;
committeeMemberCount = 4;   // or 10
```

Then deploy the contracts and use the operations CLI for the rest of the setup. **`committeeMemberCount`** should match the number of clients you run.

#### Wallet and Committee Setup

**1. Create wallets (first time only)**

```bash
./cli-operations.sh setup create-rootstock-wallets
```

**2. Fund operators (after restarting Anvil or when low on funds)**

```bash
./cli-operations.sh operator fund
```

**3. Whitelist member addresses**

Before applying to a stream, member addresses must be whitelisted on the `CommitteeRegistry` contract:

```bash
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>
```

The `CommitteeRegistry` address is in `config/base.toml` under the `CommitteeRegistry` contract entry.

**4. Apply to stream (committee setup)**

Run this after the clients are up:

```bash
./cli-operations.sh operator apply-stream -s 1
```

**Note:** Each Rootstock event needs confirmations. With Anvil auto-mining this is automatic; otherwise use `cast rpc anvil_mine N`.

## CLI Tools

- **`cli-run.sh`** — Local client launcher for development and testing
- **`cli-operations.sh`** — Operations toolkit for setup, operator, and user operations
- **`cli-infra.sh`** — Start/stop blockchains and BitVMX (Docker), and regtest remote

See [cli/README.md](cli/README.md) for details.

## Running the Union Client

### With Scripts

Typical order:

1. Have Docker running (if using Docker for blockchains/BitVMX).
2. Start the BitVMX client workspace (or use `cli-infra.sh` for local Docker).
3. Start Anvil, e.g. `anvil --block-time N`.
4. Deploy `bitvmx-union-bridge-contracts` (see their README; for local regtest: `bash ./shell/script/deploy/deploy-local.sh`).
5. Set `BASE_STORAGE_PATH` and `KEY_STORE_PASSWORD` (e.g. in `.envrc`).
6. Run the Union Client: `./cli-run.sh -h` for options.

#### Single client

```bash
./cli-run.sh
# Or with a specific ID
./cli-run.sh -i 2
```

#### Multiple clients (committee)

Use `multiclient.env` for port and path separation. You can run several clients (e.g. 4 or 10) with:

```bash
./cli-run.sh
```

#### Full workflow example

```bash
# 1. Start BitVMX client (separate terminal)
cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
rm -rf /tmp/broker_p2p* /tmp/regtest
bash run_union_example.sh

# 2. Start Anvil (separate terminal)
anvil --block-time 2

# 3. Deploy contracts (another terminal)
cd <path_to_bitvmx_union_bridge_contracts>
bash ./shell/script/deploy/deploy-local.sh

# 4. Create and fund wallets
./cli-operations.sh setup create-rootstock-wallets
./cli-operations.sh operator fund

# 5. Whitelist and apply to stream (clients must be running for apply-stream)
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>
./cli-run.sh --fresh
./cli-operations.sh operator apply-stream -s 1
```

#### Automated happy path test

With BitVMX, Anvil, Bitcoin regtest, and contracts deployed:

```bash
./cli-run.sh --start-mine
bash tests/run-happy-path.sh
# When done:
./cli-run.sh --stop-mine
```

Prerequisites: `USER_BITCOIN_WIF` and `MEMBER_BITCOIN_WIF` set; contracts deployed; background mining as above.

#### AWS Regtest (essentials)

With SSH access to the regtest instance:

```bash
./cli-infra.sh --start-regtest
# Or full fresh:
./cli-infra.sh --start-regtest --fresh

# End-to-end regtest validation
bash tests/run-happy-path-regtest.sh
```

See [regtest-instance/README.md](regtest-instance/README.md) for full details.

### With Docker

For Docker-based deployments (blockchains + BitVMX, or full operator stack), see [docker/README.md](docker/README.md).

### Development / testing

- **Mocking:** Run `./cli-mocking.sh` before `./cli-run.sh` to enable mocking (e.g. advance funds via FakePegManager).
- **Force flags:** The coordinator supports force flags in non-production (e.g. `FORCE_ADVANCE`, `FORCE_DISPUTE`). See README or coordinator docs for activation (file-based or env).

### Running crates individually

You can run each crate with Cargo; see `cli/run/src/main.rs` for the commands used by `cli-run.sh`.

## Configuration

Configuration is under `config/`, with base and environment-specific TOML files. Any value can be overridden with the `UB__` prefix: use double underscores for nesting (e.g. `UB__PROVIDER__ROOTSTOCK__URL=ws://host:4445`).

## Rootstock wallet creation (manual)

Normally done via `./cli-operations.sh setup create-rootstock-wallets`. For manual creation:

```bash
cd key-manager
cargo run --bin key-manager new-key -p <YOUR_PASSWORD> -d <PATH_TO_STORE_IT>
```

To derive public data from an existing key:

```bash
cargo run derive-public-data -p <YOUR_PASSWORD> -k <PATH_TO_FILE>
```

## CheckFork Tester — ELF and proof generation

See README section on CheckFork for generating `check_fork_args.bin`, the Stark proof, and SNARK verification (and the [ZK Proof](https://github.com/FairgateLabs/rust-bitvmx-zk-proof) repo).

## Developer conventions

This repository uses [Conventional Commits](https://www.conventionalcommits.org/). Git hooks can enforce this (see `.hooks/README.md`).

### Git hooks

```bash
cargo install rusty-hook
rusty-hook init
```

### Formatting

- **rustfmt** (nightly, for `rustfmt.toml` features):
  ```bash
  rustup component add rustfmt --toolchain nightly
  ```
- **cargo-sort** for `Cargo.toml`:
  ```bash
  cargo install cargo-sort
  ```

Configuration: [rusty-hook.toml](rusty-hook.toml).

### CI

For GitHub Actions and local testing with `act`, see [.github/WORKFLOWS.md](.github/WORKFLOWS.md).
