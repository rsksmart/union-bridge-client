# Transaction Dispatcher

For repository-level setup and workflow context, start with the [Repository README](../README.md) and the
[Contributing Guide](../CONTRIBUTING.md). This README stays focused on transaction-dispatcher ownership.

The `transaction-dispatcher` crate encapsulates the Rootstock transaction submission layer used by Union Bridge
services. Its job is to hide chain-specific transaction construction and submission behind crate APIs instead of making
other components duplicate that logic.

This crate depends on the Union Bridge contract bindings provided by the `union-contracts` crate, which is sourced from
the private `temp-rsk/bitvmx-union-bridge-contracts` repository.

Keep this README crate-scoped. User-facing HTTP routes belong to the `user-api` crate and should be documented there,
not here.
