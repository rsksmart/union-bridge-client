# Union Bridge - Monitor

# What is the Monitor?

The `monitor` checks the state of Rootstock and feeds the `zkp` with `CheckForkArgs` for zkp validation.

TODO: Add more details about the Monitor when clear.

# How to run the Monitor?

We need to specify to the `zkvm_host` the path to the `zkvm_guest`. To do so we need to set the environment variable `GUEST_CODE` with the full path to the zkvm_guest. Then, we just need to do `cargo run`.
```bash
GUEST_CODE=<base_path>/zkvm_guest cargo run
```

# Setup

This repository has [rust-bitvmx-zk-proof](https://github.com/FairgateLabs/rust-bitvmx-zk-proof) as submodule for ZK execution of `check_fork`.

Clone the repository with the `--recurse-submodules` option to automatically initialize and update the submodules:
```bash
git clone --recurse-submodules git@github.com:rsksmart/union-bridge-monitor.git
```

Alternatively, if the repository was already cloned, initialize and update the submodules with:
```bash
git submodule update --init --recursive
```