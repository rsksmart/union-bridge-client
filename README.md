# Union Bridge - Monitor

The Union Bridge Monitor is a key part of the Union Bridge Protocol. It helps connect Bitcoin and Rootstock, together
with BitVMX (through the BitVMX Client) in a trust‑minimized way. In simple terms, it watches for important events on
Rootstock and then triggers the next steps in the protocol to handle peg‑ins and peg‑outs.

## What the Monitor Does

### Event Observer

The monitor constantly scans the Rootstock blockchain for different events required for the various Union Bridge flows.
It uses **JSON‑RPC endpoints** to subscribe to new block headers and smart contract logs. Then, it extracts only the
relevant events, such as peg‑in requests and peg‑out requests. This logic is implemented under `log-indexer`
crate.

It also listens every new block produced by Rootstock, storing just the minimal required data that will also be used as
part of the different Union Bridge flows. This logic is implemented under `block-indexer` crate.

If an interruption occurs (such as a network issue), the monitor uses its saved state to resume processing. The monitor
listens for termination signals (like **SIGINT** or **SIGTERM**) and shuts down gracefully while ensuring that its
current state is saved. It also implements retry and fallback mechanisms to handle temporary connectivity problems or
blockchain reorganizations.

### Transaction Dispatcher

Based on the received events and its current state, the monitor triggers the next step in the Union Bridge protocol.

Example: When a peg‑out needs to be validated, the monitor gathers all the necessary information and passes it to the *
*check_fork module** via the **Union Client**. **(TBD: final architecture of the Union Client vs. Monitor integration is
still under discussion.)**

## Interfaces

- **Blockchain Nodes and Smart Contracts:**  
  The monitor interacts with Rootstock nodes via **JSON-RPC**, enabling it to retrieve the latest blocks, get events
  emitted by the Union Bridge contracts, verify transaction inclusion, and broadcast transactions as needed. In the long
  term, an open peer-to-peer (P2P) system could be introduced to enhance resilience against individual node failures.

- **Union Client:**  
  The Union Client is a command‑line tool (or library) that connects the monitor with other subsystems, including the
  check_fork module. **(TBD: final details of this integration and the possibility to include it in the monitor are
  under discussion.)**

- **Utilities:**  
  The repository also includes extra tools such as:
    - **Check Gaps:** A tool to verify that there are no missing blocks in the monitor’s index.
    - **Generate ELF Demo:** A utility that shows how to create the input for the check_fork function and how to produce
      Stark proofs. This demo helps illustrate how the monitor integrates with the ZKVM pipeline.

## Summary

The Union Bridge Monitor is not just a simple block indexer. It:

- **Monitors blockchain events** on Rootstock.
- **Keeps track of relevant aspects of the current protocol state.**
- **Dispatches protocol transactions** when needed.
- **Integrates with a zero‑knowledge proof pipeline** for fork validation.
- **Interfaces with the Union Bridge contracts and the Union Client** for full protocol orchestration.

# Configuration

Configuration files are located under the `config` directory, organized in environment folders. The final config is the composition of the following files in the defined order:
- `common.yaml`: common configuration for all environments.
- `{crate_name}.yaml`: specific configuration for each crate.

## Environment Variables
The project also uses some environment variables for private properties.

We recommend using `direnv` to manage them. Then you can set them up by:
1. copying `[.envrc.sample](.envrc.sample)`) in the project root as `.envrc`
2. modifying what you need
3. and running `direnv allow` (every time you do a change)
 
This will automatically load the environment variables defined in the `.envrc` on the services that require them.


# How to run the Union Client?

To run the Union-Client you have several options:
1. Manually run the required crates: `block-indexer` + `log-indexer` + `transaction-dispatcher` + `coordinator` (+ `sc-event-mocking` for mocks).
   - Make sure you have the required dependencies installed (e.g., `anvil` for mocks).
   - Use the provided sample config files under `config` to create your own configuration.
   - Run each crate with the appropriate command, passing the paths to the logger and config files.
2. Use the **sh** scripts:
   1. `./run-client.sh` to run without mocks, using **local** config.
   2. `./run-mocks.sh` in one terminal and `./run-client.sh anvil` in another one to use mocks and the **anvil** config.
3. Use the `docker-compose` file to run the Union Client. Check the [docker/README.md](docker/README.md) for more information on how to build and run the monitor using Docker.

# How to run the Monitors?

Both `log-indexer` and `block-indexer` need to be run. TBD if we create an orchestrator to run both at the same time.
Both crates are configurable, please check sample files under `config` as a reference to create your own config.

```bash
RUST_BACKTRACE=1 RUST_LOG=debug cargo run --bin log-indexer -- --logger-path "/path/to/log4rs.yaml" --config-path "/path/to/config/dir"
```

```bash
RUST_BACKTRACE=1 RUST_LOG=debug cargo run --bin block-indexer -- --logger-path "/path/to/log4rs.yaml" --config-path "/path/to/config/dir"
```

# How to run the Transaction Dispatcher?

Crate is configurable, please check sample files under `config` as a reference to create your own config.

The first time you run it, or whenever you need to create a new private key, you need to run:

```
cargo run --bin key-manager new-key -p <YOUR_PASSWORD> -d <PATH_TO_STORE_IT>
```

It will print the local path to your key, and you have to configure it in the config file.

Now you can run the transaction dispatcher providing the password to unlock the key store:

```bash
KEY_STORE_PASSWORD="<YOUR_PASSWORD>" RUST_BACKTRACE=1 RUST_LOG=debug cargo run --bin transaction-dispatcher -- --logger-path "/path/to/log4rs.yaml" --config-path "/path/to/config/dir"
```

# Utils/Check Gaps

This tool checks if there are any gaps in the blocks indexed by the monitor.

To run it:

```bash
RUST_BACKTRACE=1 RUST_LOG=debug cargo run --bin check_gaps -- --config-path "/path/to/config/dir"
```

# Utils/Generate ELF Demo

This utility shows how to generate the input for the _CheckFork_ function and its Stark Proof. Its purpose is just to
serve as reference for the integration of the new Monitor with _CheckFork_ and the ZKVM CLI. To be determined how.

## 1) Generate `check_fork_args.bin` (input to the CheckFork function)

This is the input to the _CheckFork_ function that will be executed by the `zkvm_guest` within the `zkvm_host`. To
generate it run:

```bash
cd examples/check-fork-demo
cargo run -- -o elf
```

Some instructions on how to use this file and other parameters will be printed to the console. Example:

```
CLI Args { operation: "run", fetch_start_block: 6883222, fetch_block_count: 100, cf_required_blocks: 100, cf_required_effort: 123456789, cf_init_block: 6883221, cf_init_timestamp: 1701129600 }
CheckForkArgs serialized to file: /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/util/check-fork-demo/check_fork_args.bin. Total time: 665.584µs
GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:
    - input: /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/util/check-fork-demo/check_fork_args.bin
    - elf: /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/target/riscv-guest/zkvm_guest/check_fork_guest/riscv32im-risc0-zkvm-elf/release/check_fork_guest
    - image_id: e0ce040cc1f5ab45bbadf8b81f41be224acfdb9eb7c1f39bec6102492e1137f7

```

## 2) Generate the Stark Proof

With the previous output, we can now generate the Stark Proof

```bash
cargo run --release --bin host -- prove-stark --input /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/util/check-fork-demo/check_fork_args.bin --elf /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/target/riscv-guest/zkvm_guest/check_fork_guest/riscv32im-risc0-zkvm-elf/release/check_fork_guest --output stark-proof.bin
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

## 3) Generate the Snark Proof (from the Stark) & Verify the Snark Proof

Please check the [ZKVM CLI documentation](https://github.com/FairgateLabs/rust-bitvmx-zk-proof/tree/poc-generalise-host)
for the remaining steps. Note that this doc is pointing to a WIP branch.

# Developer setup & conventions

This repository follows
the [Conventional Commits](https://www.conventionalcommits.org/en/about/#tooling-for-conventional-commits) convention,
and we have some git hooks to enforce it (check `.hooks/README.md` for more info).

Before contributing to the project, please run the following commands to set up the project:

## 1. Install _rust_ and _cargo_

https://www.rust-lang.org/tools/install

## 2. Install _rusty-hook_

This crate is used for commit hooks management.
Run the following commands to install and initialize _rusty-hook_:

```
cargo install rusty-hook
rusty-hook init
```

The file [rusty-hook.toml](rusty-hook.toml) will be used for hook configuration.