# Contributing

This guide covers contributor setup, shared runtime concepts, configuration rules, and developer conventions. Detailed
CLI and Docker commands live in the lower-level docs that sit next to the scripts they describe.

## Related Documentation

- [README.md](README.md): repository overview and documentation map.
- [cli/README.md](cli/README.md): CLI commands for local development and operations.
- [docker/README.md](docker/README.md): Docker-based local development flows.
- [docker/operator/README.md](docker/operator/README.md): local operator-focused Docker runtime flow.
- [docker/build/README.md](docker/build/README.md): Docker image build and registry operations.
- [.github/WORKFLOWS.md](.github/WORKFLOWS.md): CI workflows and local `act` usage.

## First Time Setup

Before running the Union Bridge Client for the first time, you need to complete the following setup steps:

1. **Clone the repository**
2. **Install required tools** (Rust, direnv, Foundry, etc.)
3. **Set up required repositories** (BitVMX Client, BitVMX Union Bridge Contracts)
4. **Set up environment variables** using `direnv`
5. **Configure the client** for your environment

### Clone the repository

HTTPS or SSH both work for cloning this repository. SSH is only needed if your GitHub access model for the private
contracts dependency requires it.

```bash
git clone https://github.com/rsksmart/union-bridge-client.git
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

For a working setup, you need the public `rust-bitvmx-client` repository and the private
`temp-rsk/bitvmx-union-bridge-contracts` repository. Full workspace builds, Docker image builds, and CI reproduction
require credentials for the contracts repository.

1. **BitVMX Client** - Upstream BitVMX client repository

   Clone the public [BitVMX Client](https://github.com/FairgateLabs/rust-bitvmx-client) repository and follow its
   README. Then run the BitVMX client from that checkout:

   ```bash
   git clone https://github.com/FairgateLabs/rust-bitvmx-client.git
   cd rust-bitvmx-client
   ./run_union_example.sh # this spins up the BitVMX client and make sure to have docker running on your machine
   ```

   **Note**: Make sure to have Docker running on your machine before executing the script.

2. **BitVMX Union Bridge Contracts** - Smart contracts for the Union Bridge protocol

   This private repository is required by this workspace.

   ```bash
   git clone git@github.com:temp-rsk/bitvmx-union-bridge-contracts.git
   ```

   Then follow its `README.md` for an initial setup.

### Environment Variables Setup

The project uses environment variables for both private properties and configuration overrides (see
[Configuration Files](#configuration-files) section).

#### Private Properties

The most important environment variables are:

- `BASE_STORAGE_PATH`: base path where the client will store its data (databases, keystore files, generated operator
  state, etc.). Pick a path that is writable and accessible by the user running the client.
- `KEY_STORE_PASSWORD`: password used to create and unlock Rootstock keystore files. For Docker operator flows, you can
  export it before running `cli-setup-operators.sh`, or let setup prompt for it. Setup then writes it into
  `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`.
- `USER_BITCOIN_WIF`: a Bitcoin private key WIF used for user endpoints such as peg-in and peg-out operations. You can
  generate one via the `bitcoin-wallet` with `generate_address`. For Docker operator flows, you can export it before
  running `cli-setup-operators.sh`, or let setup prompt for it. Setup then writes it into
  `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`.
  See [bitcoin-wallet README](cli/bitcoin-wallet/README.md)
  for more info.

And for in-Docker BitVMX:

- `BITCOIND_URL`: Bitcoin RPC URL used by the BitVMX client.
  **Important:** Export this in your shell before running `cli-setup-operators.sh`, because setup patches the generated
  operator YAMLs from the current environment.

We recommend using `direnv` to manage private environment variables. Then you can set them up by:

1. copying `[.envrc.sample](.envrc.sample)` in the project root as `.envrc`
2. modifying what you need. You can initially focus on the section _For local client running_.
3. and running `direnv allow` (every time you do a change)

This will automatically load the environment variables defined in the `.envrc` on the services that require them.

### Operators Setup

Use `./cli-setup-operators.sh` to set up local operators for both cargo and Docker workflows.

#### Bootstrap Local Operator State

Under the directory specified in `BASE_STORAGE_PATH`, create the base directory and then run the bootstrap helper:

```bash
mkdir -p "${BASE_STORAGE_PATH}/.union_bridge"
./cli-setup-operators.sh --ops 4
```

The bootstrap creates or reuses local Rootstock keystores, service identities, and BitVMX runtime files under
`BASE_STORAGE_PATH`, for example:

- `${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/block-indexer.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/block-indexer.pubkey_hash`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/log-indexer.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/user-api.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/coordinator.pem`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/keystore/user`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/keystore/member`
- `${BASE_STORAGE_PATH}/.union_bridge/op_1/bitvmx/keys/services.pubkey_hash`

Note: local keystores (`op_N/keystore/{member,user}`) are created by the bootstrap helper and are used by
local cargo mode (`./cli-run.sh`). Docker operator runs use container keystore paths and do not consume these
host-side cargo keystore files.

The `.pubkey_hash` files are generated from the created PEMs and are consumed by the local launcher so the
coordinator and user-api use explicit remote identities without duplicating raw hash values in
`config/env_overrides/local-committee.env`.

#### Configuring BitVMX Client (local cargo flow only)

This section applies only when you run Union Client locally with `./cli-run.sh` and manage BitVMX separately from its
own workspace.

If you are using the Docker operator flow under [`docker/operator/`](docker/operator/README.md), skip this section:
`./cli-setup-operators.sh` already generates per-operator BitVMX config copies under
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/` and patches
`components.l2.pubkey_hash` and `bitcoin.url` automatically.

The BitVMX client needs to know where to send messages back to the Union Bridge Client. You must configure the
`components.l2.pubkey_hash` in the BitVMX client config files to match the operator's Union Bridge coordinator client
identity.

**1. Read each operator's coordinator pubkey_hash**

For example:

```bash
cat ${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/coordinator.pubkey_hash
cat ${BASE_STORAGE_PATH}/.union_bridge/op_2/union-client/coordinator.pubkey_hash
```

**2. Update BitVMX client config files manually, operator by operator**

In your `rust-bitvmx-client/config/` directory, update each `config/op_N.yaml` so
`components.l2.pubkey_hash` matches the same operator's Union Bridge coordinator identity:

```yaml
components:
  l2:
    pubkey_hash: <operator-coordinator-pubkey-hash>
    id: 0
```

Examples:

- `config/op_1.yaml` -> coordinator `${BASE_STORAGE_PATH}/.union_bridge/op_1/union-client/coordinator.pubkey_hash`
- `config/op_2.yaml` -> coordinator `${BASE_STORAGE_PATH}/.union_bridge/op_2/union-client/coordinator.pubkey_hash`
- `config/op_3.yaml` -> coordinator `${BASE_STORAGE_PATH}/.union_bridge/op_3/union-client/coordinator.pubkey_hash`
- `config/op_4.yaml` -> coordinator `${BASE_STORAGE_PATH}/.union_bridge/op_4/union-client/coordinator.pubkey_hash`

This step is still manual for local BitVMX setup. Union Client and BitVMX do not share the same broker keystore.
This ensures that messages from the BitVMX client are correctly routed back to the coordinator.

#### DRP Program Files

Before running the committee setup, you must make the DRP program files accessible to the BitVMX client. These files
define the program that BitVMX will execute during the dispute resolution protocol.

The repository ships sample files under `resources/`:

| File                         | Description                                      |
|------------------------------|--------------------------------------------------|
| `resources/hello-world.elf`  | RISC-V ELF binary executed by the BitVMX CPU     |
| `resources/hello-world.yaml` | Program definition consumed by the BitVMX client |

**Steps:**

1. Set the path to the `.yaml` file in the coordinator configuration:

   ```toml
   # config/base.toml  (or your environment override file)
   [bridge.committee]
   drp_program_definition = "/path/accessible/by/bitvmx/hello-world.yaml"
   ```

2. For the normal local + Docker-backed BitVMX flow, `config/environment/local.toml` now defaults to:

    - `/app/resources/hello-world.yaml`

   which matches the BitVMX Docker mounts.

3. If you use `cli-run.sh --bitvmx-mode repo`, the launcher injects:

    - `UB__BRIDGE__COMMITTEE__DRP_PROGRAM_DEFINITION=<project_root>/resources/hello-world.yaml`

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

#### Next Steps After Bootstrap

Once the local runtime artifacts exist, the remaining local committee flow is:

1. Fund the operators:

```bash
./cli-operations.sh operator fund
```

2. Whitelist member addresses on `CommitteeRegistry`:

```bash
./cli-operations.sh operator whitelist --contract-address <COMMITTEE_REGISTRY_ADDRESS>
```

3. Run the local clients:

```bash
./cli-run.sh --fresh
# If BitVMX is running directly from repo configs, use:
# ./cli-run.sh --fresh --bitvmx-mode repo
```

4. Apply the operators to the stream:

```bash
./cli-operations.sh operator apply-stream -s 1
```

The `CommitteeRegistry` contract address can be found in `config/base.toml` under the `CommitteeRegistry` entry.

For the fuller local cargo workflow, usage examples, and command details, see [cli/README.md](cli/README.md).

## Workflow Entry Points

This file is not the command reference for every runtime path. Use it for shared setup, then drop to the lower-level
doc that owns the commands:

- local cargo workflow: [cli/README.md](cli/README.md) plus [docker/local-infra/README.md](docker/local-infra/README.md)
- Local Docker operator workflow: [docker/operator/README.md](docker/operator/README.md)
- Docker image build and registry operations: [docker/build/README.md](docker/build/README.md)

## Local Running Modes

There are 3 supported ways to run everything locally:

1. **Union Client in cargo + BitVMX in cargo**

   Use this when you want both projects running from their Rust workspaces.

   - Union Client local launcher and operations flow: [cli/README.md](cli/README.md)
   - BitVMX repo-mode setup and `cli-run.sh --bitvmx-mode repo`: [cli/README.md](cli/README.md) and
     [Configuring BitVMX Client (local cargo flow only)](#configuring-bitvmx-client-local-cargo-flow-only)

2. **Union Client in cargo + BitVMX in Docker**

   Use this when Union Client runs locally, but Bitcoin, Anvil, and BitVMX run in Docker.

   - Local infra flow: [docker/local-infra/README.md](docker/local-infra/README.md)
   - Union Client local launcher and operations flow: [cli/README.md](cli/README.md)

3. **All in Docker**

   Use this when you want BitVMX and Union Client operators running in containers.

   - Docker flow selection: [docker/README.md](docker/README.md)
   - Local operator runtime flow: [docker/operator/README.md](docker/operator/README.md)

For the Docker-backed local flows, `./cli-infra.sh` is the quickest entry point for Bitcoin, Anvil, BitVMX, and
background mining. Its local command reference lives in [docker/local-infra/README.md](docker/local-infra/README.md).

## Running the Union Client

### With Scripts

Once you have gone through the initial setup steps, the order to start up the project suite is

1. Have docker running on your local machine.
2. Start up `rust-bitvmx-client`
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

Some sub-flows in the main flows require committee collaboration. To achieve this locally, you can run several
instances of Union Client and BitVMX Client using the automated local committee setup.

The project includes a `config/env_overrides/local-committee.env` file that defines unique port numbers and
configuration paths for each client instance (1-4). This ensures no collisions between different clients for:

- Broker ports (block, log, user)
- HTTP server ports
- Database paths
- Rootstock keystore paths
- Service identity paths and broker pubkey_hash file references
- BitVMX broker ports

You can run 4 clients simultaneously using the `./cli-run.sh` script:

```bash
./cli-run.sh
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
5. Execute a pegin transaction (Bitcoin -> Rootstock) with derived x-only public key
6. Execute a pegout transaction (Rootstock -> Bitcoin) with derived compressed public key
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

- **Port Conflicts**: Each client uses unique ports defined in `config/env_overrides/local-committee.env`. Check this
  file if you encounter port issues.
- **Wallet Issues**: Re-fund wallets with `./cli-operations.sh operator fund` if needed
- **Process Cleanup**: If services fail to start due to port conflicts or corrupt database, run:
  ```bash
  pkill -f "target/debug/"
  find ${BASE_STORAGE_PATH}/.union_bridge -maxdepth 2 -type d -name database -exec rm -rf {} +
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
  --rpc-url ws://<private-regtest-rpc>:4445 \
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
local environments** (`local` and `docker`).

| Flag            | Description                                                                                                                                                                                       |
|-----------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
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

Alternatively, you can run every crate individually. Check the `cli/run/src/main.rs` file for the cargo commands used
to launch each service.

## Configuration Files

Configuration files are located under the `config` directory. The final config is the composition of the following
sources in the defined order:

- `config/base.toml`: shared configuration for all environments.
- `config/environment/{env}.toml`: environment-specific overrides (e.g. `local.toml`, `docker-local.toml`,
  `docker-alphanet.toml`).

### Configuration Overrides

Any configuration value in the TOML files can be overridden using environment variables with the `UB__` prefix. The
environment variable name should match the nested structure of the configuration, using double underscores (`__`) to
separate levels.

**Mapping Rules:**

- Nested structures use double underscores (`__`) as separators
- Arrays/lists use semicolon (`;`) as separator in environment variables

**Example TOML to Environment Variable Mapping:**

```toml
# config/base.toml
[block_broker]
ip = "127.0.0.1"
port = 5672
username = "guest"

[coordinator.database]
url = "sqlite://coordinator.db"
max_connections = 10
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

## Rootstock Wallet Creation (Manual)

This is automated by `./cli-setup-operators.sh --ops 4`, but if you want to create a wallet manually, you
can use the `key-manager` crate for that.

```bash
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

```bash
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

```text
CLI Args { operation: "elf", fixture: None, bridge_event: true, fetch_start_block: 6883222, fetch_block_count: 100, cf_required_blocks: 100, cf_required_effort: 4886718345, cf_init_block: 6883221, cf_init_timestamp: 1701129600 }
CheckForkArgs serialized to file: check-fork/tester/check_fork_args.bin. Total time: 1.79725ms
GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:
    - input: check-fork/tester/check_fork_args.bin
    - elf: target/riscv-guest/check-fork-zkp/check-fork-guest/riscv32im-risc0-zkvm-elf/release/check-fork-guest.bin
    - image_id: 18a4bad2542ac900b0681125ac38385d03139104e535590b67c473ac5465c078

```

### 2) Generate the Stark Proof

With the previous output, you can generate the Stark proof from the BitVMX
[ZK proof](https://github.com/FairgateLabs/rust-bitvmx-zk-proof/) repository.

From the `rust-bitvmx-zk-proof` repository root, point `--input` and `--elf` at paths under your
`union-bridge-client` checkout:

```bash
cargo run --release --bin host -- prove-stark \
  --input /path/to/union-bridge-client/check-fork/tester/check_fork_args.bin \
  --elf /path/to/union-bridge-client/target/riscv-guest/check-fork-zkp/check-fork-guest/riscv32im-risc0-zkvm-elf/release/check-fork-guest.bin \
  --output stark-proof.bin
```

An output like the following will be printed, showing _CheckFork_ execution result and the path to the resulting stark
proof `stark-proof.bin`.

```text
[check_fork/src/lib.rs:89:5] (cumulative_effort, required_effort) = (
    3133842214971570006248820,
    100,
)
Guest output: ACCEPT, check_fork effort: 3133842214971570006248820
The proof was executed, and the receipt saved to the file: stark-proof.bin. Total time: 128.501263917s
```

### 3) Generate the Snark Proof (from the Stark) & Verify the Snark Proof

Please check the
[SNARK proof section](https://github.com/FairgateLabs/rust-bitvmx-zk-proof?tab=readme-ov-file#snark-proof) on the
**rust-bitvmx-zk-proof** repository README for the remaining steps. Please note this is a WIP project.

## Developer Setup and Team Conventions

This repository follows the
[Conventional Commits](https://www.conventionalcommits.org/en/about/#tooling-for-conventional-commits) convention, and
we have some git hooks to enforce it (check `.hooks/README.md` for more info).

Before contributing to the project, please run the following commands to set up the project:

### 1. Install Rust and Cargo

https://www.rust-lang.org/tools/install

### 2. Install rusty-hook

This crate is used for commit hooks management. Run the following commands to install and initialize _rusty-hook_:

```bash
cargo install rusty-hook
rusty-hook init
```

### 3. Install Formatting Tools

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
