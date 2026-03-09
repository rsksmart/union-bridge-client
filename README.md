# Union Bridge - Client

> **Disclaimer**
> This project is a work in progress and should be considered experimental. It may contain bugs, security vulnerabilities, and incomplete features. Use it at your own risk -- the author(s) make no guarantees of functionality, stability, or security. Do not use this software in production environments or for handling sensitive data. Contributions, feedback, and issue reports are welcome while development is ongoing.

## Introduction

The Union Bridge Client is a Rust application that serves as a core component of the [Union Bridge](https://rootstock.io/) protocol. It connects Rootstock with [BitVMX](https://bitvmx.org/), enabling secure and trust-minimized interactions with the Bitcoin network to facilitate peg-in and peg-out flows.

In simple terms, it watches for important events on Rootstock and then triggers the next steps in the protocol to handle peg-ins (Bitcoin to Rootstock) and peg-outs (Rootstock to Bitcoin).

## Architecture

The Union Bridge Client is composed of several cooperating services:

### Event Observer

The client constantly scans the Rootstock blockchain for different events required by the Union Bridge flows. It uses **JSON-RPC endpoints** to subscribe to new block headers and smart contract logs, extracting only the relevant events such as peg-in requests and peg-out requests.

- **Block Indexer** (`block-indexer` crate): Listens to every new Rootstock block, storing the minimal required data used across the different Union Bridge flows.
- **Log Indexer** (`log-indexer` crate): Watches for specific smart contract events (logs) on Rootstock.

If an interruption occurs (such as a network issue), the client uses its saved state to resume processing. The client listens for termination signals (SIGINT, SIGTERM) and shuts down gracefully while ensuring its current state is saved. It also implements retry and fallback mechanisms to handle temporary connectivity problems or blockchain reorganizations.

### Transaction Dispatcher

The `transaction-dispatcher` crate is responsible for sending transactions to Rootstock. It manages key stores, transaction signing, and submission as required by the protocol flows.

### User API

The `user-api` crate provides a REST API for end-user interaction with the protocol, enabling operations like initiating peg-ins and peg-outs.

### Flows Coordination

The `coordinator` crate orchestrates the protocol flows and interacts with BitVMX. It manages committee setup, advance funds processing, and the overall state machine for peg-in/peg-out operations.

### Summary

The Union Bridge Client is responsible for:

- **Monitoring blockchain events** on Rootstock to detect protocol-relevant activity.
- **Maintaining protocol state**, tracking all necessary data for correct operation and recovery.
- **Dispatching protocol transactions** to Rootstock as required by protocol flows.
- **Exposing a user API** for external interaction and integration.
- **Integrating with a zero-knowledge proof pipeline** to validate blockchain forks securely.
- **Coordinating with Union Bridge contracts and BitVMX** for seamless protocol orchestration.

## Configuration

Configuration files are located under the `config` directory, organized in environment folders. The final config is the composition of base and environment-specific files.

Any configuration value can be overridden using environment variables with the `UB__` prefix. The environment variable name should match the nested structure of the configuration, using double underscores (`__`) to separate levels.

**Example:**

```yaml
# config/base.toml
provider:
  rootstock:
    url: "ws://localhost:8545"
```

Override via environment variable:

```bash
UB__PROVIDER__ROOTSTOCK__URL=ws://your-rsk-node:4445
```

## Getting Started

For instructions on setting up and running the project, see [DEVELOPER.md](DEVELOPER.md).

For Docker-based deployments, see [docker/README.md](docker/README.md).

## License

TBD
