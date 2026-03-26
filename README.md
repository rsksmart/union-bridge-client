# Union Bridge — Client

## Disclaimer

**This software is under active development.** It is **not production-ready** and has **not undergone a security audit**. It may contain bugs, vulnerabilities, and incomplete behavior. Use at your own risk. The authors make no warranties regarding functionality, stability, or security. **Do not use for production funds or sensitive data.** Issue reports and contributions are welcome.

---

## What this repository is

The **Union Bridge Client** is a Rust codebase that implements core off-chain services for the [Union Bridge](https://rootstock.io/) protocol on Rootstock. It integrates with [BitVMX](https://bitvmx.org/) and coordinates peg-in / peg-out–related flows: watching Rootstock, driving committee and BitVMX steps, and exposing APIs for operators and users.

At a glance:

| Component | Role |
|-----------|------|
| `block-indexer` | Tracks Rootstock blocks |
| `log-indexer` | Subscribes to contract events |
| `transaction-dispatcher` | Signs and submits Rootstock transactions |
| `user-api` | HTTP API for user-facing operations |
| `coordinator` | Orchestrates flows with BitVMX |

Configuration uses TOML under `config/`, overridable with `UB__`-prefixed environment variables (see [DEVELOPER.md](DEVELOPER.md)).

**License:** [MIT](LICENSE).

---

## For contributors

Use **[DEVELOPER.md](DEVELOPER.md)** for toolchain, environment, local blockchains (`docker/local-infra`), and running the client. For Docker layout and scripts (BitVMX, operators, images), see **[docker/README.md](docker/README.md)**.

## Documentation

| Topic | README |
|--------|--------|
| Developer setup, config, workflows | [DEVELOPER.md](DEVELOPER.md) |
| Docker layout (local-infra, BitVMX, operators) | [docker/README.md](docker/README.md) |
| Operator stacks (local / alphanet) | [docker/operator/README.md](docker/operator/README.md) |
| Image build & registry scripts | [docker/build/README.md](docker/build/README.md) |
| CLI scripts (`cli-run`, operations, infra) | [cli/README.md](cli/README.md) |
| Bitcoin wallet CLI | [cli/bitcoin-wallet/README.md](cli/bitcoin-wallet/README.md) |
| CheckFork / zkVM integration | [check-fork/README.md](check-fork/README.md) |
| Rootstock key tooling | [key-manager/README.md](key-manager/README.md) |
| Transaction dispatcher & contract examples | [transaction-dispatcher/README.md](transaction-dispatcher/README.md) |
| BitVMX operator keys (per environment) | [local](docker/bitvmx-client/config/local/keys/README.md), [regtest](docker/bitvmx-client/config/regtest/keys/README.md), [testnet](docker/bitvmx-client/config/testnet/keys/README.md), [alphanet](docker/bitvmx-client/config/alphanet/keys/README.md) |
| Commit hooks & formatting | [.hooks/README.md](.hooks/README.md) |
