# Union Bridge - Monitor

The Union Bridge Monitor is a key part of the Union Bridge Protocol. It helps connect Bitcoin and Rootstock in a trust‑minimized way. In simple terms, it watches for important events on Rootstock and then triggers the next steps in the protocol to handle peg‑ins and peg‑outs.

## What the Monitor Does

### Event Observer
The monitor constantly scans the Rootstock blockchain for peg‑in and peg‑out requests. It uses **JSON‑RPC endpoints** to subscribe to new block headers and smart contract logs. Then, it extracts only the relevant events, such as peg‑in requests and peg‑out requests.

### State Keeper
The monitor keeps an internal record of the protocol’s state — for example, which peg‑slots are active. This state is stored using a storage backend (either cached or persisted). When new blocks or transaction confirmations are detected, the monitor updates its state. If an interruption occurs (such as a network issue), the monitor uses its saved state to resume processing.

The monitor listens for termination signals (like **SIGINT** or **SIGTERM**) and shuts down gracefully while ensuring that its current state is saved. It also has retry and fallback mechanisms to handle temporary connectivity problems or blockchain reorganizations.

### Transaction Dispatcher
Based on the events it sees and its current state, the monitor triggers the next step in the Union Bridge protocol. For example, it might collect signatures and send a peg‑out kick‑off transaction.

When a peg‑out needs to be validated, the monitor gathers all the necessary information and passes it to the **check_fork module** via the **Union Client**. **(TBD: final architecture of the Union Client vs. Monitor integration is still under discussion.)**

## Interfaces

- **Blockchain Nodes and Smart Contracts:**  
  The monitor interacts with Rootstock nodes via **JSON-RPC**, enabling it to retrieve the latest blocks, get events emitted by the Union Bridge contracts, verify transaction inclusion, and broadcast transactions as needed. In the long term, an open peer-to-peer (P2P) system could be introduced to enhance resilience against individual node failures.

- **Union Client:**  
  The Union Client is a command‑line tool (or library) that connects the monitor with other subsystems, including the check_fork module. **(TBD: final details of this integration and the possibility to include it in the monitor are under discussion.)**

- **Utilities:**  
  The repository also includes extra tools such as:
  - **Check Gaps:** A tool to verify that there are no missing blocks in the monitor’s index.
  - **Generate ELF Demo:** A utility that shows how to create the input for the check_fork function and how to produce Stark proofs. This demo helps illustrate how the monitor integrates with the ZKVM pipeline.

## Summary

The Union Bridge Monitor is not just a simple block indexer. It:
- **Monitors blockchain events** on Rootstock.
- **Keeps track of the current protocol state.**
- **Dispatches protocol transactions** when needed.
- **Integrates with a zero‑knowledge proof pipeline** for fork validation.
- **Interfaces with the Union Bridge contracts and the Union Client** for full protocol orchestration.


# How to run the Monitor?

```bash
cd monitor
RUST_BACKTRACE=1 RUST_LOG=debug cargo run
```

# Utils/Check Gaps

This tool checks if there are any gaps in the blocks indexed by the monitor.

To run it:

```bash
cd utils/check_gaps
RUST_BACKTRACE=1 RUST_LOG=debug cargo run
```

# Utils/Generate ELF Demo

This utility shows how to generate the input for the _CheckFork_ function and its Stark Proof. Its purpose is just to
serve as reference for the integration of the new Monitor with _CheckFork_ and the ZKVM CLI. To be determined how.

## 1) Generate `check_fork_args.bin` (input to the CheckFork function)

This is the input to the _CheckFork_ function that will be executed by the `zkvm_guest` within the `zkvm_host`. To
generate it run:

```bash
cd utils/generate_elf_demo
cargo run
```

Some instructions on how to use this file and other parameters will be printed to the console. Example:

```
get_blocks done, total blocks '100'
CheckForkArgs serialized to file: /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/util/generate_elf_demo/check_fork_args.bin. Total time: 665.584µs
GetBlocks executed and CheckForkArgs generated. Relevant parameters for the interaction with the ZKVM CLI:
    - input: /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/util/generate_elf_demo/check_fork_args.bin
    - elf: /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/target/riscv-guest/zkvm_guest/check_fork_guest/riscv32im-risc0-zkvm-elf/release/check_fork_guest
    - image_id: e0ce040cc1f5ab45bbadf8b81f41be224acfdb9eb7c1f39bec6102492e1137f7

```

## 2) Generate the Stark Proof

With the previous output, we can now generate the Stark Proof

```bash
cargo run --release --bin host -- prove-stark --input /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/util/generate_elf_demo/check_fork_args.bin --elf /Users/illuque/workspace/rootstock/union_bridge/union-bridge-monitor/target/riscv-guest/zkvm_guest/check_fork_guest/riscv32im-risc0-zkvm-elf/release/check_fork_guest --output stark-proof.bin
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