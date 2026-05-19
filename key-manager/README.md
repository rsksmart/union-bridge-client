# Key Manager

For repository-level setup and workflow context, start with the [Repository README](../README.md) and the
[Local Setup Guide](../LOCAL_SETUP.md). This README only covers the crate-specific commands below.

Run these commands from the repository root:

```bash
cargo run --bin key-manager -- --help
```

Generate a new key pair:

```bash
cargo run --bin key-manager -- new-key -p test -d <PATH_TO_STORE_IT>
```

Derive the public data from an existing key:

```bash
cargo run --bin key-manager -- derive-public-data -p test -k <PATH_TO_KEY>
```

Alternative storage backends may exist in upstream libraries, but they are not implemented in this crate.
