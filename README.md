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
git clone --recurse-submodules git@github.com:rsksmart/union-bridge-client.git
```

The `--recurse-submodules` flag is essential as it automatically initializes and updates all required submodules.

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
   git clone git@github.com:rsksmart/bitvmx-union-bridge-contracts.git
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
- `WALLET_PRIVATE_KEY`: a Bitcoin private key WIF. You can generate one via the `bitcoin-wallet` with `generate_address`.
  See [bitcoin-wallet README](bitcoin-wallet/README.md) for more info.

We recommend using `direnv` to manage private environment variables. Then you can set them up by:

1. copying `[.envrc.sample](.envrc.sample)`) in the project root as `.envrc`
2. modifying what you need. You can initially focus on the section _For local client running_.
3. and running `direnv allow` (every time you do a change)

This will automatically load the environment variables defined in the `.envrc` on the services that require them.

### Multi Client Setup

The Multi Client setup is mostly automated using the `run-cli` tool. But we need to run 2 manual steps.

#### Creating the base directory

Under the directory specified in the `BASE_STORAGE_PATH` env, run the following command to create the base directory:

```
mkdir -p .union_bridge
```

Note: The `keystore` subdirectory will be created automatically when you create wallets.

#### Configuring the Committee

You will need to tweak the committee size and requirements according to the committee you want to run. For example, to
use a committee
of 4 members, 2 Watchtowers (aka Verifiers) and 2 Operators (aka Provers) like mentioned in **Combined Setup**, you will
need to edit
`bitvmx-union-bridge-contracts/src/CommitteeRegistry.sol` and change:

   ```
   minCommitteeWatchtowers = 3;
   minCommitteeOperators = 3;
   committeeMemberCount = 10;
   ```

to:

   ```
   minCommitteeWatchtowers = 2;
   minCommitteeOperators = 2;
   committeeMemberCount = 4;
   ```

Then you should deploy the contracts, and you can use `run-cli` wallet setup for the rest of the setup.

**N.B**
Please note that the `committeeMemberCount` value should always match the number of clients you intend to spin up

**1. Create Wallets (first time only)**

This creates wallets for each multi-client instance. Each client gets **two wallets**: one for member operations and one for user operations.

By default, this creates wallets for 10 clients (20 wallets total). You can specify a different number with `--num-wallets`.

This is required for the **Transaction Dispatcher** crate to be able to sign transactions and send them to Rootstock.

```bash
./run-cli.sh setup-wallets create --num-wallets 10
```

**2. Fund Wallets (every time you restart Anvil or if you run out of funds)**

This funds both the member and user wallets for each client created in the previous step.

```bash
./run-cli.sh setup-wallets fund --num-wallets 10
```

**3. Setup Committee (optional)**

See this [section](#temporary-bitvmx-funding-process-for-cargo-running-mode) for more information on a temporary
approach to fund BitVMX.

This creates a Committee so of 4 members (by default) so you can test the committee collaboration flows.

```bash
./multiclient-setup.sh --committee-setup 4 --stream_id 1  # for 4 clients (hint: the clients need to be running for the `--committee-setup` flag to execute)
```

Note that each RSK event in the flow (3 atm) needs to reach the required confirmations. If you started anvil with auto
mining, it will happen at some point; otherwise you can manually mine blocks with `cast rpc anvil_mine N`.

**Combined Setup**

This combines the wallet creation and funding steps to have wallets ready for your clients.

```bash
./run-cli.sh setup-wallets both --num-wallets 4
```

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
6. Run the Union Client in the mode you want. Run `./run-client.sh --help` to better understand the available
   options.

#### Running a Single Client

You can run a single instance of the Union Client using:

```bash
# Single client mode (defaults to CLIENT_ID=1
./run-client.sh --features anvil

# Single client with specific ID
./run-client.sh --id 2 --features anvil
```

#### Running Multiple Clients (Committee Collaboration)

Some sub-flows in the main flows require committee collaboration. To achieve this locally, you can run several instances
of Union Client and BitVMX Client using the automated multiclient setup.

The project includes a `multiclient.env` file that defines unique port numbers and configuration paths for each client
instance (1-10). This ensures no collisions between different clients for:

- Broker ports (block, log, user)
- HTTP server ports
- Database paths
- Keystore paths
- BitVMX broker ports

You can run up to 10 clients using the `./run-client.sh` script.

```bash
./run-client.sh --num-clients 4 --features anvil
```

#### Complete Workflow Example

Here's the complete workflow to set up and run 4 clients:

```bash
# 1. Start BitVMX client (in separate terminal)
cd <path_to_bitvmx_workspace_repo>/rust-bitvmx-client
./run_union_example.sh

# 2. Start Anvil and deploy contracts (in separate terminal)
anvil
# Deploy contracts in another terminal
# Fund bitcoin wallets (checkout docker-integrated/ README)

# 3. Run the clients
./run-client.sh --num-clients 4 --features anvil

# 4. Set up multiclient wallets
./run-cli.sh setup-wallets both --num-wallets 4

# 5. Set up committee (optional, requires clients to be running)
./multiclient-setup.sh --committee-setup 4 --stream_id 1
```

#### Troubleshooting

- **Port Conflicts**: Each client uses unique ports defined in `multiclient.env`, take this into account if you edit it
  or create more clients
- **Wallet Issues**: Use `./run-cli.sh setup-wallets fund --num-wallets 4` to refund wallets if needed
- **Process Cleanup**: If services fail to start due to port conflicts or corrupt database, run
  `pkill -f "target/debug/" && rm -rf ${BASE_STORAGE_PATH}/.union_bridge/database` to clean up processes, but take into
  account that this
  will kill all Rust processes running and prune your database
- **Local Setup Issues**: Check that `BASE_STORAGE_PATH` env var is correctly set and accessible

### With Docker

Use the `docker-compose` file to run the Union Client. Check the [docker/README.md](docker/README.md) for more
information on how to build and run the client using Docker.

### Development/Testing Setup

Optionally, you can run `./run-mocks.sh` in another terminal before `./run-client.sh ...`. This will:

- spin up a mocked BitVMX client
- spin up an anvil node to simulate the Rootstock blockchain
- deploy the BitVMX Union Bridge contracts on the anvil node

### Individual Crates using Cargo

Alternatively, you can run every crate individually, just check `./run-client.sh` for the commands used to run each
crate.

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

This is automated in the `run-cli` wallet setup commands, but if you want to create a wallet manually, you can use the
`key-manager` crate for that.

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

## QA-tools/Generate ELF Demo

This utility shows how to generate the input for the _CheckFork_ function and its Stark Proof. Its purpose is just to
serve as reference for the integration of the new Client with _CheckFork_ and the ZKVM CLI. To be determined how.

### 1) Generate `check_fork_args.bin` (input to the CheckFork function)

This is the input to the _CheckFork_ function that will be executed by the `zkvm_guest` within the `zkvm_host`. To
generate it run:

```bash
cd qa-tools/check-fork
cargo run --bin check_fork_runner -- -o elf
```

Some instructions on how to use this file and other parameters will be printed to the console. Example:

```
CLI Args { operation: "elf", fixture: None, bridge_event: true, fetch_start_block: 6883222, fetch_block_count: 100, cf_required_blocks: 100, cf_required_effort: 4886718345, cf_init_block: 6883221, cf_init_timestamp: 1701129600 }
CheckForkArgs serialized to file: /path/to/repo/union-bridge-client/qa-tools/check_fork_args.bin. Total time: 3.741667ms
GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:
    - input: /path/to/repo/union-bridge-client/qa-tools/check_fork_args.bin
    - elf: /path/to/repo/union-bridge-client/qa-tools/target/riscv-guest/methods/check-fork-guest/riscv32im-risc0-zkvm-elf/release/check-fork-guest.bin
    - image_id: c24b36840af78835ddca7eb7ddc933d2b1bcc01656133b2c110b42102fc71f3c

```

### 2) Generate the Stark Proof

With the previous output, we can now generate the Stark Proof
Clone Fairgate's [ZK Proof](https://github.com/FairgateLabs/rust-bitvmx-zk-proof/) repo, for now at
`poc-generalise-host` branch.

Then run the following command where:

```bash
cargo run --release --bin host -- prove-stark --input /Users/illuque/workspace/rootstock/union_bridge/union-bridge-client/util/check-fork-demo.old/check_fork_args.bin --elf /Users/illuque/workspace/rootstock/union_bridge/union-bridge-client/target/riscv-guest/zkvm_guest/check_fork_guest/riscv32im-risc0-zkvm-elf/release/check_fork_guest --output stark-proof.bin
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

Please check the [ZKVM CLI documentation](https://github.com/FairgateLabs/rust-bitvmx-zk-proof/tree/poc-generalise-host)
for the remaining steps. Note that this doc is pointing to a WIP branch.

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

The file [rusty-hook.toml](rusty-hook.toml) will be used for hook configuration.

### GitHub Actions

To test locally the GitHub Actions, you can use the `act` tool. You need to have Docker installed and running on your
machine, as `act` uses Docker to run the actions in a local environment.

You can install it via Homebrew:

```bash
brew install act
```

**Only the first time you run `act`, or whenever the base image changes**. To do so, run the following command from the
`.github/act` directory:

Then, copy the `.actrc.sample` to `.actrc` and configure it as needed. This file is used to configure the `act` tool.

To run the same actions as the CI runs on pull requests, you can use the following command:

```bash
act pull_request -s KEY_STORE_FILE=$(cat <path_to_your_keystore_file>) --container-architecture linux/amd64
```

To run just Crate Tests, you can use the following command:

```bash
act -j crates-tests -s KEY_STORE_FILE=$(cat <path_to_your_keystore_file>) --container-architecture linux/amd64
```

To run just QA Tests, you can use the following command:

```bash
act -j qa-tests -s KEY_STORE_FILE=$(cat <path_to_your_keystore_file>) --container-architecture linux/amd64
```

NOTE: Uploading and downloading artifacts is slow locally, but fast on the CI.
NOTE: You can add `--reuse` to reuse previous Docker containers to speed up execution by skipping setup and preserving
cache, filesystem, and environment state.
NOTE: If you find concurrency errors, try running with `--concurrent-jobs 1` to run the actions sequentially.

## Temporary BitVMX funding process for cargo running mode

This is a temporary process to fund the BitVMX Bitcoin wallets for the Committee Setup.

With the 4 clients started, run in bash:

```
for port in 40001 40002 40003 40004; do
  echo "GET http://0.0.0.0:$port/bitvmx-address"
  curl -sS -X GET "http://0.0.0.0:$port/bitvmx-address"
  echo
done
```

This will print in the logs 4 Bitcoin addresses (one per operator) that you will use in the next steps.

Then, through the `bitcoin-wallet` crate (started with `cargo run --release`) you should run:

1. `clear_db`
2. `mine_utxo 9000000000`
3. `send_to_address <btc_addr_1>,<btc_addr_2>,<btc_addr_3>,<btc_addr_4> 25000000`
