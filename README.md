# Union Bridge - Client

The Union Bridge Client is a key part of the Union Bridge Protocol. It helps connect Bitcoin and Rootstock, together
with BitVMX, in a trust-minimized way. In practice, it watches Rootstock for protocol-relevant events and triggers the
next steps needed to execute peg-in and peg-out flows.

## Disclaimer

This project is a work in progress and should be considered experimental. It may contain bugs, security
vulnerabilities, and incomplete features.

Use it at your own risk. The author(s) make no guarantees of functionality, stability, or security.

Do not use this software in production environments or for handling sensitive data.

Contributions, feedback, and issue reports are welcome while development is ongoing.

## Client Overview

The Union Bridge Client is a Rust application that connects Rootstock with BitVMX and implements the client-side logic
required by the Union Bridge protocol. It observes Rootstock by subscribing to new block headers and smart contract logs
through JSON-RPC, filtering only the events that matter for the protocol and persisting the minimum state needed to
recover from interruptions or chain reorganizations; this part is implemented in the `log-indexer` and
`block-indexer` crates. It also sends transactions to Rootstock through the `transaction-dispatcher` crate,
which centralizes contract interactions, key usage, and transaction submission. User-facing operations,
including requesting peg-in addresses and peg-outs, are exposed through the `user-api` crate. The end-to-end
coordination of the different multi-step flows is handled by the `coordinator` crate, which connects blockchain events,
contract state, broker messaging, BitVMX interactions, and timeout handling.

## Documentation Map

| If you need to...                                                           | Read                                                                                                                                                                                                                                         |
|-----------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| get the recommended local setup, shared env rules, and contributor workflow | [Contributing Guide](CONTRIBUTING.md)                                                                                                                                                                                                        |
| use the local wrappers and operations CLI                                   | [CLI Tools Guide](cli/README.md)                                                                                                                                                                                                             |
| choose a Docker flow                                                        | [Docker Guide](docker/README.md)                                                                                                                                                                                                             |
| run local blockchains and BitVMX in Docker                                  | [Local Infra Guide](docker/local-infra/README.md)                                                                                                                                                                                            |
| run local operators in Docker                                               | [Operator Docker Runtime Guide](docker/operator/README.md)                                                                                                                                                                                   |
| build or publish Docker images                                              | [Docker Build Guide](docker/build/README.md)                                                                                                                                                                                                 |
| read detailed e2e flow documentation                                        | [E2E Flow Documentation](docs/e2e/README.md)                                                                                                                                                                                                 |
| inspect crate-specific detail                                               | nearby component READMEs such as [CheckFork Guide](check-fork/README.md), [Transaction Dispatcher Guide](transaction-dispatcher/README.md), [Key Manager Guide](key-manager/README.md), and [Wallet CLI Guide](cli/bitcoin-wallet/README.md) |

## Contributing

Contributor setup, shared configuration, local runtime modes, and the recommended development path live in the
[Contributing Guide](CONTRIBUTING.md).

## License

This project is licensed under the [MIT License](LICENSE).
