# Transaction Dispatcher

The `transaction-dispatcher` project is a Rust-based application designed to handle and process various Union Bridge transactions.

This project depends on the **Union Bridge Contract Bindings** for interaction with the Smart Contract. These are provided by the `union-contracts` crate (check `Cargo.toml`), which points to [FairgateLabs/bitvmx-union-bridge-contracts](https://github.com/FairgateLabs/bitvmx-union-bridge-contracts). `forge-bind` is used under the hood to generate the bindings.

Proper version handling for the `union-contracts` dependency is crucial to ensure compatibility with the Union Bridge Smart Contracts. Using a fixed tag or version (instead of branch reference) helps maintain stability, avoid unexpected changes, and ensure reproducibility across environments.