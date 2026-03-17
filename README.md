# Union Bridge - Client

> **Disclaimer**  
> This project is a work in progress and should be considered experimental. It may contain bugs, security vulnerabilities, and incomplete features. Use at your own risk — the authors make no guarantees of functionality, stability, or security. Do not use in production or for sensitive data. Contributions and issue reports are welcome.

## Introduction

The Union Bridge Client is a Rust application that serves as a core component of the [Union Bridge](https://rootstock.io/) protocol. It connects Rootstock with [BitVMX](https://bitvmx.org/), enabling secure, trust-minimized interactions with the Bitcoin network for peg-in and peg-out flows.

In short, it watches for relevant events on Rootstock and triggers the next steps in the protocol.

## Architecture

The client is made up of several services:

- **Block indexer** (`block-indexer`): Listens to new Rootstock blocks and stores minimal data needed for Union Bridge flows.
- **Log indexer** (`log-indexer`): Subscribes to smart contract events (e.g. peg-in/peg-out requests) on Rootstock.
- **Transaction dispatcher** (`transaction-dispatcher`): Signs and sends transactions to Rootstock.
- **User API** (`user-api`): REST API for end-user operations (e.g. initiating peg-in/peg-out).
- **Coordinator** (`coordinator`): Orchestrates flows and talks to BitVMX (committee setup, advance funds, peg-in/peg-out state).

The client persists state, handles SIGINT/SIGTERM for clean shutdown, and uses retries and fallbacks for connectivity and reorgs.

## Configuration

Configuration lives under `config/` (base + environment TOML files). Any option can be overridden with the `UB__` prefix and double underscores for nesting.

**Example:** `UB__PROVIDER__ROOTSTOCK__URL=ws://your-rsk-node:4445`

## Getting Started

For setup, tooling, and running the client: **[DEVELOPER.md](DEVELOPER.md)**.

For Docker (blockchains, BitVMX, full operator stack): **[docker/README.md](docker/README.md)**.

## License

TBD
