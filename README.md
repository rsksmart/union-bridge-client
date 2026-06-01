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

## Quick Start

Main local commands, run from the repo root after the one-time setup in the [Local Setup Guide](docs/LOCAL_SETUP.md):

```bash
bash scripts/run-infra.sh --start   # start local blockchains, BitVMX, and mining (Docker)
bash scripts/run-clients.sh         # run the operator client services locally
bash scripts/test-flows.sh          # run the automated end-to-end happy-path flows
```

For funding, peg-in/peg-out, and wallet operations (`scripts/operations.sh`, `scripts/bitcoin-wallet.sh`), see the
[CLI Tools Guide](cli/README.md). The full recommended path lives in the [Local Setup Guide](docs/LOCAL_SETUP.md).

## Documentation Map

| If you need to...                                                            | Read                                                                                                                                                                                                                                         |
|------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| get the recommended local setup, shared env rules, and runtime modes         | [Local Setup Guide](docs/LOCAL_SETUP.md)                                                                                                                                                                                                          |
| understand engineering standards and commit conventions                      | [Contributing Guide](CONTRIBUTING.md)                                                                                                                                                                                                        |
| install or troubleshoot git hooks                                            | [Local Setup Guide](docs/LOCAL_SETUP.md#install-git-hooks) and [Hooks Guide](.hooks/README.md)                                                                                                                                                    |
| follow AI-agent guidance, project routing, and verification commands         | [Agent Guide](AGENTS.md)                                                                                                                                                                                                                     |
| use the local wrappers and operations CLI                                    | [CLI Tools Guide](cli/README.md)                                                                                                                                                                                                             |
| choose a Docker flow                                                         | [Docker Guide](docker/README.md)                                                                                                                                                                                                             |
| run local blockchains and BitVMX in Docker                                   | [Local Infra Guide](docker/local-infra/README.md)                                                                                                                                                                                            |
| run local operators in Docker                                                | [Operator Docker Runtime Guide](docker/operator/README.md)                                                                                                                                                                                   |
| build or publish Docker images                                               | [Docker Build Guide](docker/build/README.md)                                                                                                                                                                                                 |
| read detailed e2e flow documentation                                         | [E2E Flow Documentation](docs/e2e/README.md)                                                                                                                                                                                                 |
| inspect crate-specific detail                                                | nearby component READMEs such as [CheckFork Guide](crates/check-fork/README.md), [Transaction Dispatcher Guide](crates/transaction-dispatcher/README.md), [Key Manager Guide](crates/key-manager/README.md), and [Wallet CLI Guide](cli/bitcoin-wallet/README.md) |

## Contributing

Engineering standards and commit conventions live in the [Contributing Guide](CONTRIBUTING.md).
Local setup, hook installation, shared configuration, runtime modes, and the recommended development path live in the
[Local Setup Guide](docs/LOCAL_SETUP.md). Per-hook details live in the [Hooks Guide](.hooks/README.md).

## License

This project is licensed under the [MIT License](LICENSE).
