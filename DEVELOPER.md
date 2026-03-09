# Developer Guide

This guide covers setting up and running the Union Bridge Client for development and testing.

## First Time Setup

Before running the Union Bridge Client for the first time:

1. **Clone the repository**
2. **Install required tools** (Rust, direnv, Foundry, etc.)
3. **Set up required repositories** (BitVMX Workspace, BitVMX Union Bridge Contracts)
4. **Set up environment variables** using `direnv`
5. **Configure the client** for your environment

### Clone the Repository

```bash
git clone git@github.com:rsksmart/union-bridge-client.git
```

### Tooling

#### Required Tools

1. **Rust and Cargo** - The project is built in Rust
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **direnv** - For managing environment variables
   ```bash
   # macOS
   brew install direnv

   # Add to your shell profile (~/.zshrc or ~/.bashrc)
   eval "$(direnv hook zsh)"  # or bash
   ```

3. **Foundry** - Ethereum development toolkit (includes Anvil)
   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

#### Optional Tools

- **Docker** - For containerized deployment (see [docker/README.md](docker/README.md))
- **act** - For running GitHub Actions locally
  ```bash
  brew install act
  ```

### Required Repositories

1. **BitVMX Workspace** - Contains the BitVMX client

   Clone the [BitVMX Workspace](https://github.com/FairgateLabs/rust-bitvmx-workspace) repository and follow
   the README instructions to set it up. Then run the BitVMX client:

   ```bash
   git clone git@github.com:FairgateLabs/rust-bitvmx-workspace.git
   cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
   ./run_union_example.sh
   ```

   **Note**: Make sure to have Docker running on your machine before executing the script.

2. **BitVMX Union Bridge Contracts** - Smart contracts for the Union Bridge protocol
   ```bash
   git clone git@github.com:temp-rsk/bitvmx-union-bridge-contracts.git
   ```

   Then follow its `README.md` for an initial setup.

### Environment Variables Setup

The project uses environment variables for both private properties and configuration overrides (see
[Configuration](README.md#configuration) section in README.md).

#### Private Properties

The most important environment variables:

- `KEY_STORE_PASSWORD`: password used to create/unlock Rootstock wallets
- `BASE_STORAGE_PATH`: base path where the client stores its data (databases, keystore files, etc.)
- `WALLET_PRIVATE_KEY`: a Bitcoin private key WIF. Generate one via `bitcoin-wallet` with `generate_address`.
  See [bitcoin-wallet README](cli/bitcoin-wallet/README.md) for more info.

We recommend using `direnv` to manage private environment variables:

1. Copy [.envrc.sample](.envrc.sample) to `.envrc` in the project root
2. Modify the values you need (focus on the _For local client running_ section initially)
3. Run `direnv allow` (every time you make a change)

### Multi Client Setup

The Multi Client setup is mostly automated using `cli-operations.sh`.

#### Creating the Base Directory

Under the directory specified in `BASE_STORAGE_PATH`:

```bash
mkdir -p .union_bridge
```

The `keystore` subdirectory will be created automatically.

#### Configuring the Committee

You will need to tweak the committee size and requirements. For example, for a committee of 4 members (2 Watchtowers + 2 Operators), edit `bitvmx-union-bridge-contracts/src/CommitteeRegistry.sol`:

```solidity
minCommitteeWatchtowers = 2;
minCommitteeOperators = 2;
committeeMemberCount = 4;
```

Then deploy the contracts and use the operations CLI for the rest of the setup.

**Note:** `committeeMemberCount` should match the number of clients you intend to run (currently hardcoded to 4 in the CLI).

#### Wallet and Committee Setup

**1. Create Wallets (first time only)**

```bash
./cli-operations.sh setup create-rootstock-wallets
```

**2. Fund Operators (every time you restart Anvil or run out of funds)**

```bash
./cli-operations.sh operator fund
```

**3. Apply to Stream (committee setup)**

```bash
./cli-operations.sh operator apply-stream -s 1
```

**Note:** Each Rootstock event requires confirmations. With Anvil auto-mining, this happens automatically.
Otherwise, manually mine blocks with `cast rpc anvil_mine N`.

## CLI Tools

The project includes two CLI tools for local development and operations:

- **`cli-run.sh`**: Local client launcher for development and testing
- **`cli-operations.sh`**: Operations toolkit for setup, operator, and user operations

For detailed documentation, see [cli/README.md](cli/README.md).

## Running the Union Client

### With Scripts

Once you have gone through the initial setup steps, the order to start up the project:

1. Have Docker running on your local machine.
2. Start the BitVMX client workspace.
3. Start Anvil, optionally with auto mining: `anvil --block-time N`
4. Deploy the `bitvmx-union-bridge-contracts`. See corresponding `README.md`.
5. Make `BASE_STORAGE_PATH` and `KEY_STORE_PASSWORD` available (set them in `.envrc`).
6. Run the Union Client. Run `./cli-run.sh -h` for available options.

#### Running a Single Client

```bash
# Single client mode (defaults to CLIENT_ID=1)
./cli-run.sh

# Single client with specific ID
./cli-run.sh -i 2
```

#### Running Multiple Clients (Committee Collaboration)

Some sub-flows require committee collaboration. The project includes `multiclient.env` which defines unique port numbers and configuration paths for each client instance (1-4).

```bash
./cli-run.sh
```

#### Complete Workflow Example

```bash
# 1. Start BitVMX client (in separate terminal)
cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
rm -rf /tmp/broker_p2p* ; rm -rf /tmp/regtest ; bash run_union_example.sh

# 2. Start Anvil (in separate terminal)
anvil --block-time 2

# 3. Deploy contracts (in another terminal)
cd <path_to_bitvmx_union_bridge_contracts>
bash ./shell/script/deploy/deploy-local.sh

# 4. Create and fund wallets
./cli-operations.sh setup create-rootstock-wallets
./cli-operations.sh operator fund

# 5. Run the 4 clients
./cli-run.sh --fresh

# 6. Apply operators to stream (requires clients to be running)
./cli-operations.sh operator apply-stream -s 0
```

#### Automated Happy Path Test

Once you have the setup running (steps 1-3 above), you can run a fully automated end-to-end test:

```bash
# Prerequisites:
# - BitVMX client running
# - Anvil running
# - Bitcoin regtest node running with RPC enabled
# - Contracts deployed
# - USER_BITCOIN_WIF and MEMBER_BITCOIN_WIF environment variables set
# - Background mining running (start with: ./cli-run.sh --start-mine)

# Start background mining
./cli-run.sh --start-mine

# Run automated happy path test
bash tests/run-happy-path.sh

# Stop background mining when done
./cli-run.sh --stop-mine
```

This test will automatically:

1. Prepare wallets (clear databases, mine initial UTXOs)
2. Fund operator wallets (Bitcoin + Rootstock)
3. Apply operators to stream
4. Execute a pegin transaction (Bitcoin to Rootstock) with derived x-only public key
5. Execute a pegout transaction (Rootstock to Bitcoin) with derived compressed public key
6. Verify pegout completion in coordinator logs

> **Note**: The user-api endpoints require public keys in the request body. The test script derives these from
> `USER_BITCOIN_WIF` using `bitcoin-cli`. For manual testing:
>
> ```bash
> # Derive x-only public key (32 bytes) for pegin
> bitcoin-cli -regtest getdescriptorinfo "wpkh($USER_BITCOIN_WIF)" | jq -r '.descriptor' | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/0x\1/' | cut -c1-2,5-
>
> # Derive compressed public key (33 bytes) for pegout
> bitcoin-cli -regtest getdescriptorinfo "wpkh($USER_BITCOIN_WIF)" | jq -r '.descriptor' | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/0x\1/'
> ```

#### Troubleshooting

- **Port Conflicts**: Each client uses unique ports defined in `multiclient.env`.
- **Wallet Issues**: Re-fund wallets with `./cli-operations.sh operator fund` if needed.
- **Process Cleanup**: If services fail to start:
  ```bash
  pkill -f "target/debug/"
  rm -rf ${BASE_STORAGE_PATH}/.union_bridge/database
  ```
  **Warning:** This kills all Rust processes and prunes your database.
- **Local Setup Issues**: Verify `BASE_STORAGE_PATH` and `KEY_STORE_PASSWORD` environment variables.

### With Docker

For Docker-based deployments, see [docker/README.md](docker/README.md) which covers:

- **Local development**: Running blockchains + BitVMX in Docker while developing Union Client with cargo
- **Full operator deployment**: Running everything (BitVMX + Union Client) in Docker
- **Building images**: Creating and pushing Union Client Docker images

### Development/Testing Setup

Optionally, run `./cli-mocking.sh` before starting clients with `./cli-run.sh` to enable mocking.

#### Mocking Advance Funds Events via FakePegManager

If run right after the contracts deployment (on a clean Anvil instance), the correct address should already be set in the config.

Available commands:

- `raf` or `invoke-request-advance-funds`: start monitoring blocks for advance funds (emits RequestAdvanceFunds event)
    - copy the printed `pegout_id` for the next step
- `kaf` or `invoke-advance-funds`: generate a fake advance-funds event
    - provide the `pegout_id` from the previous step

#### Force Flags for Testing

The coordinator supports force flags to trigger specific behaviors during testing. These flags are **only active in
non-production environments** (Local, LocalDocker, Regtest) and are automatically disabled in Alphanet and Testnet.

| Flag | Description |
|------|-------------|
| `FORCE_ADVANCE` | Contains a Rootstock address. The targeted operator skips the signature sub-flow, simulating operator misbehavior. Since signatures never complete, the advance funds timeout triggers naturally. |
| `FORCE_DISPUTE` | Overrides the `ReimbursementResult` challenge result to `OperatorWon`, simulating a successful dispute. |

**Activation methods:**

1. **File-based (recommended - hot-reloadable):**
   ```bash
   # Enable flags
   echo "0xOPERATOR_ADDRESS" > /tmp/FORCE_ADVANCE
   touch /tmp/FORCE_DISPUTE

   # Disable flags (remove files)
   rm /tmp/FORCE_ADVANCE
   rm /tmp/FORCE_DISPUTE
   ```

2. **Environment variables (set at startup):**
   ```bash
   FORCE_ADVANCE=0xOPERATOR_ADDRESS FORCE_DISPUTE=true ./cli-run.sh
   ```

**Hot-reloading:** The file-based approach allows QA to toggle flags while the coordinator is running. New flows will
immediately pick up the change without restarting the application.

### Individual Crates using Cargo

Alternatively, run every crate individually. Check `cli/run/src/main.rs` for the cargo commands used to launch each service.

## Rootstock Wallet Creation (manual)

This is automated in `cli-operations.sh setup create-rootstock-wallets`, but for manual creation:

```bash
cd key-manager
cargo run --bin key-manager new-key -p <YOUR_PASSWORD> -d <PATH_TO_STORE_IT>
```

To derive public information from an existing key:

```bash
cd key-manager
cargo run derive-public-data -p <YOUR_PASSWORD> -k <PATH_TO_FILE>
```

## CheckFork Tester - Generate ELF Demo

This utility shows how to generate the input for the _CheckFork_ function and its Stark Proof.

### 1) Generate `check_fork_args.bin`

```bash
cd check-fork/tester
cargo run --bin check-fork-tester -- -o elf
```

### 2) Generate the Stark Proof

Clone [ZK Proof](https://github.com/FairgateLabs/rust-bitvmx-zk-proof/) repo and run:

```bash
cargo run --release --bin host -- prove-stark \
  --input <path_to>/check_fork_args.bin \
  --elf <path_to>/check-fork-guest.bin \
  --output stark-proof.bin
```

### 3) Generate the SNARK Proof & Verify

See the [SNARK proof section](https://github.com/FairgateLabs/rust-bitvmx-zk-proof?tab=readme-ov-file#snark-proof)
in the **rust-bitvmx-zk-proof** repository README.

## Developer Conventions

This repository follows
the [Conventional Commits](https://www.conventionalcommits.org/en/about/#tooling-for-conventional-commits) convention.
Git hooks are configured to enforce this (check `.hooks/README.md` for more info).

### Setup Git Hooks

```bash
cargo install rusty-hook
rusty-hook init
```

### Formatting Tools

Install `rustfmt` nightly (supports features in `rustfmt.toml` like imports reorder and grouping):

```bash
rustup component add rustfmt --toolchain nightly
```

Install `cargo-sort` to sort dependencies in `Cargo.toml` files:

```bash
cargo install cargo-sort
```

The file [rusty-hook.toml](rusty-hook.toml) configures the git hooks.

### GitHub Actions

For information about CI workflows, including how to test locally with `act`, see [.github/WORKFLOWS.md](.github/WORKFLOWS.md).
