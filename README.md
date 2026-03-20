# Union Bridge - Client

The Union Bridge Client is a key part of the Union Bridge Protocol. It helps connect Bitcoin and Rootstock, together
with BitVMX (through the BitVMX Client) in a trust‑minimized way. In simple terms, it watches for important events on
Rootstock and then triggers the next steps in the protocol to handle peg‑ins and peg‑outs.

## Introduction

The Union Bridge Client is a Rust application that serves as a core component of the Union Bridge protocol. Its goal is
to connect Rootstock with BitVMX, enabling secure and trust-minimized interactions with the Bitcoin network to
facilitate the different flows of the Union Bridge protocol.

Below is a high-level summary of the core responsibilities handled by the Union Bridge Client.

### Event Observer

The client constantly scans the Rootstock blockchain for different events required for the various Union Bridge flows.
It uses **JSON‑RPC endpoints** to subscribe to new block headers and smart contract logs. Then, it extracts only the
relevant events, such as peg‑in requests and peg‑out requests. This logic is implemented under `log-indexer` crate.

It also listens every new block produced by Rootstock, storing just the minimal required data that will also be used as
part of the different Union Bridge flows. This logic is implemented under `block-indexer` crate.

If an interruption occurs (such as a network issue), the client uses its saved state to resume processing. The client
listens for termination signals (like **SIGINT** or **SIGTERM**) and shuts down gracefully while ensuring that its
current state is saved. It also implements retry and fallback mechanisms to handle temporary connectivity problems or
blockchain reorganizations.

### Transaction Dispatcher

Implemented under the `transaction-dispatcher` crate, this component is responsible for sending transactions to
Rootstock.

TODO: better document this

### User API

Implemented under the `user-api` crate, this component provides a user-friendly API for end user interaction with the
protocol.

TODO: better document this

### Flows Coordination

Implemented under the `coordinator` crate, this component orchestrates the different flows of the Union and interacts
with BitVMX.

TODO: better document this

### Summary

The Union Bridge Client is responsible for:

- **Monitoring blockchain events** on Rootstock to detect protocol-relevant activity.
- **Maintaining protocol state**, tracking all necessary data for correct operation and recovery.
- **Dispatching protocol transactions** to Rootstock as required by protocol flows.
- **Exposing a user API** for external interaction and integration.
- **Integrating with a zero‑knowledge proof pipeline** to validate blockchain forks securely.
- **Coordinating with Union Bridge contracts and the Union Client** for seamless protocol orchestration.

## First Time Setup

Before running the Union Bridge Client for the first time, you need to complete the following setup steps:

1. **Clone the repository with SSH** (required for submodules access)
2. **Install required tools** (Rust, direnv, Foundry, etc.)
3. **Set up required repositories** (BitVMX Workspace, BitVMX Union Bridge Contracts)
4. **Set up environment variables** using `direnv`
5. **Configure the client** for your environment

### Clone the repository

**Important**: You must clone the repository using SSH because the project uses private submodules that require SSH
authentication.

```bash
git clone git@github.com:rsksmart/union-bridge-client.git
```

### Tooling

Before running the Union Bridge Client, you need to install and set up the following tools and repositories:

#### Required Tools

1. **Rust and Cargo** - The project is built in Rust
   ```bash
   # Install from https://www.rust-lang.org/tools/install
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
   # Install Foundry
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

   You have to clone the [BitVMX Workspace](https://github.com/FairgateLabs/rust-bitvmx-workspace) repository and follow
   the README instructions to set it up. Once done, you can run the BitVMX client by following the next steps:

   ```bash
   git clone git@github.com:FairgateLabs/rust-bitvmx-workspace.git
   cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
   ./run_union_example.sh # this spins up the BitVMX client and make sure to have docker running on your machine
   ```

   **Note**: Make sure to have Docker running on your machine before executing the script.

2. **BitVMX Union Bridge Contracts** - Smart contracts for the Union Bridge protocol
   ```bash
   git clone git@github.com:temp-rsk/bitvmx-union-bridge-contracts.git
   ```

   Then follow its `README.md` for an initial setup.

### Environment Variables Setup

The project uses environment variables for both private properties and configuration overrides (see
[Configuration Files](#configuration-files) section).

#### Private Properties

The most important environment variables that need to be exported when using the scripts mentioned in this README (and
the Union Client) are:

- `KEY_STORE_PASSWORD`: password that will be used to create the Rootstock wallets (automatic) and to
  unlock the corresponding keystore files when running the client (see [Multi Client Setup](#Multi-Client-Setup) below)
- `BASE_STORAGE_PATH`: base path where the client will store its data (databases, keystore files, etc.). Pick a path
  that is writable and accessible by the user running the client.
- `WALLET_PRIVATE_KEY`: a Bitcoin private key WIF. You can generate one via the `bitcoin-wallet` with
  `generate_address`.
  See [bitcoin-wallet README](cli/bitcoin-wallet/README.md) for more info.

We recommend using `direnv` to manage private environment variables. Then you can set them up by:

1. copying `[.envrc.sample](.envrc.sample)`) in the project root as `.envrc`
2. modifying what you need. You can initially focus on the section _For local client running_.
3. and running `direnv allow` (every time you do a change)

This will automatically load the environment variables defined in the `.envrc` on the services that require them.

### Multi Client Setup

The Multi Client setup is mostly automated using the `cli-operations.sh` tool. You need to complete a few manual steps
first.

#### Creating the base directory

Under the directory specified in the `BASE_STORAGE_PATH` env, run the following command to create the base directory:

```bash
mkdir -p ${BASE_STORAGE_PATH}/.union_bridge/keystore
```

#### Creating Broker Identities

The Union Bridge Client uses explicit broker identities in local multi-client mode. Each operator creates separate
broker identities for:

- `block-indexer` broker server
- `log-indexer` broker server
- `user-api` broker server
- `coordinator` broker client

```bash
./cli-operations.sh setup create-broker-identities
```

This command creates or reuses stable files under `BASE_STORAGE_PATH`, for example:

- `${BASE_STORAGE_PATH}/.union_bridge/broker/block-indexer/multi-client-1.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/broker/block-indexer/multi-client-1.pubkey_hash`
- `${BASE_STORAGE_PATH}/.union_bridge/broker/log-indexer/multi-client-1.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/broker/user-api/multi-client-1.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/broker/coordinator/multi-client-1.pem`

The `.pubkey_hash` files are generated from the created PEMs and are consumed by the local launcher so the
coordinator and user-api use explicit remote identities without duplicating raw hash values in `multiclient.env`.
The command also prints the coordinator `pubkey_hash` for each operator so you can copy the correct value into the
matching local BitVMX Client config.

#### Configuring BitVMX Client

The BitVMX client needs to know where to send messages back to the Union Bridge Client. You must configure the
`components.l2.pubkey_hash` in the BitVMX client config files to match the operator's Union Bridge coordinator client
identity.

**1. Read each operator's coordinator pubkey_hash**

For example:

```bash
cat ${BASE_STORAGE_PATH}/.union_bridge/broker/coordinator/multi-client-1.pubkey_hash
cat ${BASE_STORAGE_PATH}/.union_bridge/broker/coordinator/multi-client-2.pubkey_hash
```

**2. Update BitVMX client config files manually, operator by operator**

In your `rust-bitvmx-workspace/rust-bitvmx-client/config/` directory, update each `config/op_N.yaml` so
`components.l2.pubkey_hash` matches the same operator's Union Bridge coordinator identity:

```yaml
components:
  l2:
    pubkey_hash: <operator-coordinator-pubkey-hash>
    id: 0
```

Examples:

- `config/op_1.yaml` -> coordinator `multi-client-1.pubkey_hash`
- `config/op_2.yaml` -> coordinator `multi-client-2.pubkey_hash`
- `config/op_3.yaml` -> coordinator `multi-client-3.pubkey_hash`
- `config/op_4.yaml` -> coordinator `multi-client-4.pubkey_hash`

This step is still manual for local BitVMX setup. Union Client and BitVMX do not share the same broker keystore.
This ensures that messages from the BitVMX client are correctly routed back to the coordinator.

#### DRP Program Files

Before running the committee setup, you must make the DRP program files accessible to the BitVMX client. These files
define the program that BitVMX will execute during the dispute resolution protocol.

The repository ships sample files under `resources/`:

| File | Description |
|---|---|
| `resources/hello-world.elf` | RISC-V ELF binary executed by the BitVMX CPU |
| `resources/hello-world.yaml` | Program definition consumed by the BitVMX client |

**Steps:**

1. Copy (or symlink) both files to a path that is accessible by the BitVMX client process.
2. Set the path to the `.yaml` file in the coordinator configuration:

   ```toml
   # config/base.toml  (or your environment override file)
   [bridge.committee]
   drp_program_definition = "/path/accessible/by/bitvmx/hello-world.yaml"
   ```

   Alternatively, export the corresponding environment variable:

   ```bash
   export UB__BRIDGE__COMMITTEE__DRP_PROGRAM_DEFINITION="/path/accessible/by/bitvmx/hello-world.yaml"
   ```


#### Configuring the Committee

You will need to tweak the committee size and requirements according to the committee you want to run. For example, to
use a committee of 4 members, 2 Watchtowers (aka Verifiers) and 2 Operators (aka Provers), you will need to edit
`bitvmx-union-bridge-contracts/src/CommitteeRegistry.sol` and change:

```solidity
minCommitteeWatchtowers = 3;
minCommitteeOperators = 3;
committeeMemberCount = 10;
```

to:

```solidity
minCommitteeWatchtowers = 2;
minCommitteeOperators = 2;
committeeMemberCount = 4;
```

Then you should deploy the contracts, and you can use the operations CLI for the rest of the setup.

**Note:** The `committeeMemberCount` value should always match the number of clients you intend to run (currently
hardcoded to 4 in the CLI).

#### Wallet and Committee Setup

**1. Create Wallets (first time only)**

Creates Rootstock wallets for 4 operators. Each operator gets **two wallets**: one for member operations and one for
user operations (8 wallets total).

This is required for the **Transaction Dispatcher** to sign and send transactions to Rootstock.

```bash
./cli-operations.sh setup create-rootstock-wallets
```

**2. Fund Operators (every time you restart Anvil or run out of funds)**

Funds both Bitcoin addresses and Rootstock wallets for all operators.

```bash
./cli-operations.sh operator fund
```

**3. Whitelist Member Addresses**

Before operators can apply to a stream, their member addresses must be whitelisted on the `CommitteeRegistry` contract.
This is required by the contract to control which addresses are allowed to participate in committees.

```bash
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>
```

The `CommitteeRegistry` contract address can be found in `config/base.toml` under the `CommitteeRegistry` entry.

**4. Apply to Stream (committee setup)**

Applies all 4 operators to a stream to form the committee. The clients must be running before executing this command.

```bash
./cli-operations.sh operator apply-stream -s 1
```

**Note:** Each Rootstock event in the flow requires confirmations. With anvil auto-mining, this happens automatically.
Otherwise, manually mine blocks with `cast rpc anvil_mine N`.

## CLI Tools

The project includes two CLI tools for local development and operations:

- **`cli-run.sh`**: Local client launcher for development and testing
- **`cli-operations.sh`**: Operations toolkit for setup, operator, and user operations

For detailed documentation, usage examples, and command references, see [cli/README.md](cli/README.md).

## AWS Regtest (Essentials)

Access to regtest requires SSH credentials for:
- `ubuntu@union-bridge-use2-1.regtest.rskcomputing.net`

Run from repository root:

```bash
# Fast path: start operators with existing config/addresses
./cli-infra.sh --start-regtest

# Full fresh orchestration (rebuild + reconfigure + restart)
./cli-infra.sh --start-regtest --fresh

# End-to-end regtest validation
bash tests/run-happy-path-regtest.sh
```

Command summary:
- `--start-regtest`: starts operators with existing deployed addresses/config.
- `--start-regtest --fresh`: runs full remote fresh orchestration and clean operator restart.

Important: if you change branch on the regtest host (`~/union-bridge-client`), rebuild images tagged as `latest-regtest` before starting operators:

```bash
cd docker/build
bash d-compose-cli.sh build --tag=latest-regtest --no-cache
```

For full instance details (hosts, env vars, artifacts, validation, troubleshooting), see:
- [`regtest-instance/README.md`](regtest-instance/README.md)

## Running the Union Client

### With Scripts

Once you have gone through the initial setup steps, the order to start up the project suite is

1. Have docker running on your local machine.
2. Start up the rust-bitvmx-client-workspace
3. Start anvil, opt. with auto mining with `anvil --block-time N` (where `N` is the number of seconds between blocks)
4. Deploy the `bitvmx-union-bridge-contracts`. See corresponding `README.md`. (Hint: for local regtest deployment use
   `bash ./shell/script/deploy/deploy-local.sh`)
5. Make available `BASE_STORAGE_PATH` and `KEY_STORE_PASSWORD` environment variables (you can set them in your
   `.envrc` file)
6. Run the Union Client in the mode you want. Run `./cli-run.sh -h` to better understand the available
   options.

#### Running a Single Client

You can run a single instance of the Union Client using:

```bash
# Single client mode (defaults to CLIENT_ID=1)
./cli-run.sh

# Single client with specific ID
./cli-run.sh -i 2
```

#### Running Multiple Clients (Committee Collaboration)

Some sub-flows in the main flows require committee collaboration. To achieve this locally, you can run several instances
of Union Client and BitVMX Client using the automated multiclient setup.

The project includes a `multiclient.env` file that defines unique port numbers and configuration paths for each client
instance (1-4). This ensures no collisions between different clients for:

- Broker ports (block, log, user)
- HTTP server ports
- Database paths
- Rootstock keystore paths
- Broker identity paths and broker pubkey_hash file references
- BitVMX broker ports

You can run 4 clients simultaneously using the `./cli-run.sh` script:

```bash
./cli-run.sh
```

#### Complete Workflow Example

Here's the complete workflow to set up and run 4 clients:

```bash
# 1. Start BitVMX client (in separate terminal)
cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
rm -rf /tmp/broker_p2p* ; rm -rf /tmp/regtest ; bash run_union_example.sh

# 2. Start Anvil (in separate terminal)
anvil --block-time 2  # optional: auto-mine every 2 seconds

# 3. Deploy contracts (in another terminal)
cd <path_to_bitvmx_union_bridge_contracts>
bash ./shell/script/deploy/deploy-local.sh

# 4. Create and fund wallets
./cli-operations.sh setup create-rootstock-wallets
./cli-operations.sh setup create-broker-identities
./cli-operations.sh operator fund

# 5. Whitelist member addresses (uses CommitteeRegistry address from config/base.toml)
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>

# 6. Run the 4 clients
./cli-run.sh --fresh

# 7. Apply operators to stream (requires clients to be running)
./cli-operations.sh operator apply-stream -s 0
```

#### Automated Happy Path Test

Once you have the setup running (steps 1-3 above), you can run a fully automated end-to-end test that exercises the
complete flow:

```bash
# Prerequisites:
# - BitVMX client running
# - Anvil running
# - Bitcoin regtest node running with RPC enabled
# - Contracts deployed
# - USER_BITCOIN_WIF and MEMBER_BITCOIN_WIF environment variables set (for deriving public keys)
# - Background mining running (start with: ./cli-run.sh --start-mine)

# Start background mining (in a separate terminal or before running the test)
./cli-run.sh --start-mine

# Run automated happy path test
bash tests/run-happy-path.sh

# Stop background mining when done
./cli-run.sh --stop-mine
```

This test will automatically:

1. Prepare wallets (clear databases, mine initial UTXOs)
2. Fund operator wallets (Bitcoin + Rootstock)
3. Whitelist member addresses on CommitteeRegistry
4. Apply operators to stream
5. Execute a pegin transaction (Bitcoin → Rootstock) with derived x-only public key
6. Execute a pegout transaction (Rootstock → Bitcoin) with derived compressed public key
7. Verify pegout completion in coordinator logs

The test includes comprehensive health checks to detect issues early.

> **Note**: The user-api endpoints now require public keys to be passed in the request body instead of deriving them
> server-side from `USER_BITCOIN_WIF`. The test script automatically derives these keys from `USER_BITCOIN_WIF` using
> `bitcoin-cli`. For manual testing, you can derive the keys with:
>
> ```bash
> # Derive x-only public key (32 bytes) for pegin - returns 0x + 64 hex chars
> bitcoin-cli -regtest getdescriptorinfo "wpkh($USER_BITCOIN_WIF)" | jq -r '.descriptor' | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/0x\1/' | cut -c1-2,5-
>
> # Derive compressed public key (33 bytes) for pegout - returns 0x + 66 hex chars
> bitcoin-cli -regtest getdescriptorinfo "wpkh($USER_BITCOIN_WIF)" | jq -r '.descriptor' | sed -E 's/^wpkh\(([0-9a-fA-F]+)\)#.*/0x\1/'
> ```

#### Troubleshooting

- **Port Conflicts**: Each client uses unique ports defined in `multiclient.env`. Check this file if you encounter port
  issues.
- **Wallet Issues**: Re-fund wallets with `./cli-operations.sh operator fund` if needed
- **Process Cleanup**: If services fail to start due to port conflicts or corrupt database, run:
  ```bash
  pkill -f "target/debug/"
  rm -rf ${BASE_STORAGE_PATH}/.union_bridge/database
  ```
  **Warning:** This kills all Rust processes and prunes your database.
- **Local Setup Issues**: Verify `BASE_STORAGE_PATH` and `KEY_STORE_PASSWORD` environment variables are set correctly

### With Docker

For Docker-based deployments, see [docker/README.md](docker/README.md) which covers:

- **Local development**: Running blockchains + BitVMX in Docker while developing Union Client with cargo
- **Full operator deployment**: Running everything (BitVMX + Union Client) in Docker
- **Building images**: Creating and pushing Union Client Docker images

### Development/Testing Setup

Optionally, you can run `./cli-mocking.sh` in another terminal before starting the clients with `./cli-run.sh`. This
will:

#### Mocking Advance Funds Events via FakePegManager

By default (`deploy` mode), the CLI deploys `FakePegManager` against local anvil.

For regtest, use `attach` mode (`--no-deploy`) and pass the predeployed address:

```bash
./cli-mocking.sh \
  --rpc-url ws://node-use2-1.regtest.rskcomputing.net:4445 \
  --fake-peg-manager-address 0x... \
  --no-deploy
```

You can also provide values through env vars:

- `MOCKS_PRIVATE_KEY`
- `FAKE_PEG_MANAGER_ADDRESS`
- `CHECK_FORK_REQUIRED_NUM_BLOCKS` (optional, defaults to `5`)

You will have the following commands available:

- `raf` or `invoke-request-advance-funds`: start monitoring blocks for advance funds (emits RequestAdvanceFunds event)
    - copy the printed `pegout_id`, you will need it for the next step
- `kaf` or `invoke-advance-funds`: generate a fake advance-funds event that triggers the advance funds in Coordinator
    - you need to provide the `pegout_id` from the previous step

(check cli help for more info)

#### Force Flags for Testing

The coordinator supports force flags to trigger specific behaviors during testing. These flags are **only active in
non-production environments** (Local, LocalDocker, Regtest) and are automatically disabled in Alphanet and Testnet.

| Flag | Description |
|------|-------------|
| `FORCE_ADVANCE` | Contains a Rootstock address. The targeted operator skips the signature sub-flow, simulating operator misbehavior. Since signatures never complete, the advance funds timeout triggers naturally. |

**Activation methods:**

1. **File-based (recommended - hot-reloadable):**
   ```bash
   # Enable flags
   echo "0xOPERATOR_ADDRESS" > /tmp/FORCE_ADVANCE

   # Disable flags (remove files)
   rm /tmp/FORCE_ADVANCE
   ```

2. **Environment variables (set at startup):**
   ```bash
   FORCE_ADVANCE=0xOPERATOR_ADDRESS ./cli-run.sh
   ```

**Hot-reloading:** The file-based approach allows QA to toggle flags while the coordinator is running. New flows will
immediately pick up the change without restarting the application.

### Individual Crates using Cargo

Alternatively, you can run every crate individually. Check the `cli/run/src/main.rs` file for the cargo commands used to
launch each service.

## Configuration Files

Configuration files are located under the `config` directory, organized in environment folders. The final config is the
composition of the following files in the defined order:

- `common.yaml`: common configuration for all environments.
- `{crate_name}.yaml`: specific configuration for each crate.

### Configuration Overrides

Any configuration value in the YAML files can be overridden using environment variables with the `UB__` prefix. The
environment variable name should match the nested structure of the configuration, using double underscores (`__`) to
separate levels.

**Mapping Rules:**

- Nested structures use double underscores (`__`) as separators
- Arrays/lists use semicolon (`;`) as separator in environment variables

**Example YAML to Environment Variable Mapping:**

```yaml
# config/common.yaml
block_broker:
  ip: "127.0.0.1"
  port: 5672
  username: "guest"

coordinator:
  database:
    url: "sqlite://coordinator.db"
    max_connections: 10
```

Corresponding environment variables:

```bash
UB__block_broker__ip=127.0.0.1
UB__block_broker__port=5672
UB__block_broker__username=guest
UB__coordinator__database__url=sqlite://coordinator.db
UB__coordinator__database__max_connections=10
```

This approach allows for flexible configuration management across different deployment environments without modifying
configuration files.

## Rootstock Wallet creation (manual)

This is automated in the `cli-operations.sh setup create-rootstock-wallets` command, but if you want to create a wallet
manually, you can use the `key-manager` crate for that.

```
cd key-manager
cargo run --bin key-manager new-key -p <YOUR_PASSWORD> -d <PATH_TO_STORE_IT>
```

This will output:

- the local path to your key: you will have to set it in the corresponding `transaction-dispatcher.yaml` config file
- the public key
- the address (this will be automatically used by the wallet setup commands)

Keep track of the password you used, as you will need to set it up in `KEY_STORE_PASSWORD` env var. Check the
[Environment Variables Setup](#environment-variables-setup) section for more information on how to set it up.

You can always derive the public information of the key afterward if you remember the password by running the following
command:

```
cd key-manager
cargo run derive-public-data -p <YOUR_PASSWORD> -k <PATH_TO_FILE>
```

## CheckFork Tester - Generate ELF Demo

This utility shows how to generate the input for the _CheckFork_ function and its Stark Proof. Its purpose is just to
serve as a reference for the integration of the new Client with _CheckFork_ and the ZKVM CLI. In a real scenario, we
won't use the CLI but a programmatic approach via BitVMX Api (see `IncomingBitVMXApiMessages::GenerateZKP` usages in
Coordinator crate).

### 1) Generate `check_fork_args.bin` (input to the CheckFork function)

This is the input to the _CheckFork_ function that will be executed by the `zkvm_guest` within the `zkvm_host`. To
generate it run:

```bash
cd check-fork/tester
cargo run --bin check-fork-tester -- -o elf
```

Some instructions on how to use this file and other parameters will be printed in the console. Example:

```
CLI Args { operation: "elf", fixture: None, bridge_event: true, fetch_start_block: 6883222, fetch_block_count: 100, cf_required_blocks: 100, cf_required_effort: 4886718345, cf_init_block: 6883221, cf_init_timestamp: 1701129600 }
CheckForkArgs serialized to file: /Users/illuque/workspace/union-bridge/union-bridge-client/check-fork/tester/check_fork_args.bin. Total time: 1.79725ms
GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:
    - input: /Users/illuque/workspace/union-bridge/union-bridge-client/check-fork/tester/check_fork_args.bin
    - elf: /Users/illuque/workspace/union-bridge/union-bridge-client/target/riscv-guest/check-fork-zkp/check-fork-guest/riscv32im-risc0-zkvm-elf/release/check-fork-guest.bin
    - image_id: 18a4bad2542ac900b0681125ac38385d03139104e535590b67c473ac5465c078

```

### 2) Generate the Stark Proof

With the previous output, we can now generate the Stark Proof
Clone Fairgate's [ZK Proof](https://github.com/FairgateLabs/rust-bitvmx-zk-proof/) repo or use Workspace one - main
branch works ATM.

Then run the following command where:

```bash
cargo run --release --bin host -- prove-stark --input /Users/illuque/workspace/union-bridge/union-bridge-client/check-fork/tester/check_fork_args.bin --elf /Users/illuque/workspace/union-bridge/union-bridge-client/target/riscv-guest/check-fork-zkp/check-fork-guest/riscv32im-risc0-zkvm-elf/release/check-fork-guest.bin --output stark-proof.bin
```

An output like the following will be printed, showing _CheckFork_ execution result and the path to the resulting stark
proof `stark-proof.bin`.

```
[/Users/illuque/.cargo/git/checkouts/union-bridge-check-fork-47c61d4052b7ed6f/6d36b88/check_fork/src/lib.rs:89:5] (cumulative_effort, required_effort) = (
    3133842214971570006248820,
    100,
)
Guest output: ACCEPT, check_fork effort: 3133842214971570006248820
The proof was executed, and the receipt saved to the file: stark-proof.bin. Total time: 128.501263917s
```

### 3) Generate the Snark Proof (from the Stark) & Verify the Snark Proof

Please check
the [SNARK proof section](https://github.com/FairgateLabs/rust-bitvmx-zk-proof?tab=readme-ov-file#snark-proof) on 
**rust-bitvmx-zk-proof** repository README for the remaining steps. Please note this is a WIP project.

## Developer setup & team conventions

This repository follows
the [Conventional Commits](https://www.conventionalcommits.org/en/about/#tooling-for-conventional-commits) convention,
and we have some git hooks to enforce it (check `.hooks/README.md` for more info).

Before contributing to the project, please run the following commands to set up the project:

## 1. Install _rust_ and _cargo_

https://www.rust-lang.org/tools/install

### 2. Install _rusty-hook_

This crate is used for commit hooks management.
Run the following commands to install and initialize _rusty-hook_:

```
cargo install rusty-hook
rusty-hook init
```

## Install formatting tools

Install `rustfmt` nightly, as it supports features we use in `rustfmt.toml` like imports reorder and grouping:

```bash
rustup component add rustfmt --toolchain nightly
```

Install `cargo-sort` to sort dependencies in `Cargo.toml` files:

```bash
cargo install cargo-sort
```

Together with the hooks, these tools will help you keep the codebase clean and consistent on `pre-commit`.

The file [rusty-hook.toml](rusty-hook.toml) will be used for hook configuration.

### GitHub Actions

For information about the GitHub Actions workflows in this project, including how to test them locally with `act`, see
[.github/WORKFLOWS.md](.github/WORKFLOWS.md).
