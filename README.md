# Union Bridge - Client

The Union Bridge Client is a key part of the Union Bridge Protocol. It helps connect Bitcoin and Rootstock, together
with BitVMX (through the BitVMX Client) in a trust-minimized way. In simple terms, it watches for important events on
Rootstock and then triggers the next steps in the protocol to handle peg-ins and peg-outs.

## Introduction

The Union Bridge Client is a Rust application that serves as a core component of the Union Bridge protocol. Its goal is
to connect Rootstock with BitVMX, enabling secure and trust-minimized interactions with the Bitcoin network to
facilitate the different flows of the Union Bridge protocol.

Below is a high-level summary of the core responsibilities handled by the Union Bridge Client.

### Event Observer

The client constantly scans the Rootstock blockchain for different events required for the various Union Bridge flows.
It uses **JSON-RPC endpoints** to subscribe to new block headers and smart contract logs. Then, it extracts only the
relevant events, such as peg-in requests and peg-out requests. This logic is implemented under `log-indexer` crate.

It also listens every new block produced by Rootstock, storing just the minimal required data that will also be used as
part of the different Union Bridge flows. This logic is implemented under `block-indexer` crate.

If an interruption occurs (such as a network issue), the client uses its saved state to resume processing. The client
listens for termination signals (like **SIGINT** or **SIGTERM**) and shuts down gracefully while ensuring that its
current state is saved. It also implements retry and fallback mechanisms to handle temporary connectivity problems or
blockchain reorganizations.

### Transaction Dispatcher

Implemented under the `transaction-dispatcher` crate, this component is responsible for sending transactions to
Rootstock. It wraps contract interactions, key usage, and transaction submission so the rest of the system can request
protocol actions without duplicating chain-specific logic.

### User API

Implemented under the `user-api` crate, this component provides a user-friendly API for end user interaction with the
protocol. It exposes the entry points used for operations such as peg-ins and peg-outs and validates the request data
needed by downstream flows.

### Flows Coordination

Implemented under the `coordinator` crate, this component orchestrates the different flows of the Union and interacts
with BitVMX. It ties together blockchain events, contract state, broker messaging, and timeout handling for
multi-step protocol execution.

### Summary

The Union Bridge Client is responsible for:

- **Monitoring blockchain events** on Rootstock to detect protocol-relevant activity.
- **Maintaining protocol state**, tracking all necessary data for correct operation and recovery.
- **Dispatching protocol transactions** to Rootstock as required by protocol flows.
- **Exposing a user API** for external interaction and integration.
- **Integrating with a zero-knowledge proof pipeline** to validate blockchain forks securely.
- **Coordinating with Union Bridge contracts and the Union Client** for seamless protocol orchestration.

## Documentation Map

Higher-level docs route to lower-level ones:

| If you need to... | Read |
| --- | --- |
| understand contributor setup, shared configuration, and developer conventions | [CONTRIBUTING.md](CONTRIBUTING.md) |
| run the client locally or use the operations wrappers | [cli/README.md](cli/README.md) |
| choose a Docker workflow | [docker/README.md](docker/README.md) |
| run full operators in Docker | [docker/operator/README.md](docker/operator/README.md) |
| build or publish Docker images | [docker/build/README.md](docker/build/README.md) |
| dive into component-specific detail | component READMEs close to the implementation, such as [check-fork/README.md](check-fork/README.md), [transaction-dispatcher/README.md](transaction-dispatcher/README.md), [key-manager/README.md](key-manager/README.md), and [cli/bitcoin-wallet/README.md](cli/bitcoin-wallet/README.md) |

### E2E Documentation

For detailed end-to-end flow documentation, see [docs/e2e/README.md](docs/e2e/README.md).

## Contributing

- [CONTRIBUTING.md](CONTRIBUTING.md): contributor setup, local and regtest flows, runtime configuration, and
  developer-oriented documentation.

## License

This project is licensed under the [MIT License](LICENSE).
