# Union Bridge - Monitor

# What is the Monitor?

TODO: Add more details about the Monitor.

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