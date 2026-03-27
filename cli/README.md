# Union Bridge CLI Tools

The project includes two CLI tools in the `cli/` workspace for managing local development and operator operations.

## Related Docs

- [../CONTRIBUTING.md](../CONTRIBUTING.md): contributor setup, shared configuration, and local validation flows
- [../docker/operator/README.md](../docker/operator/README.md): Docker operator runtime flow
- [bitcoin-wallet/README.md](bitcoin-wallet/README.md): Bitcoin wallet helper used by some operations

## Workflow Entry Points

- Local cargo workflow: bootstrap with `./cli-setup-operators.sh --env local --ops 4`, then use the commands below.
  The broader sequence lives in [../CONTRIBUTING.md](../CONTRIBUTING.md).
- Docker operator workflow: use the same operations commands with `--env local-docker`, `--env alphanet`, or
  `--env testnet` after following [../docker/operator/README.md](../docker/operator/README.md).

## `cli-run.sh` - Local Client Runner

Launches one or more Union Bridge clients locally for development and testing.

```bash
./cli-run.sh --help

# Run all 4 clients
./cli-run.sh --features anvil

# Run a single client
./cli-run.sh --id 1 --features anvil

# Run with fresh databases
./cli-run.sh --fresh --features anvil

# Select BitVMX source
./cli-run.sh --bitvmx-mode docker   # default, containers use .union_bridge/op_N/bitvmx/keys/services.pubkey_hash
./cli-run.sh --bitvmx-mode repo     # running from cloned repo, ignores UB__COORDINATOR__BITVMX__PUBKEY_HASH_FILE_N override in [local-committee.env](../config/env_overrides/local-committee.env) and uses config/base.toml hash (matches bitvmx repo value)

# View logs from all 4 coordinators
./cli-run.sh --logs

# Kill all existing running services
./cli-run.sh --kill

# Start background mining (Anvil every 1s, Bitcoin every 5s)
./cli-run.sh --start-mine

# Stop background mining
./cli-run.sh --stop-mine
```

### Options

- `--help`: Display help message and exit.
- `--id`, `-i`: Run a single client with the specified ID (1-4). If not provided, runs 4 clients.
- `--features`, `-f`: Optional features to pass to cargo (e.g. "anvil").
- `--fresh`: Start with clear databases (removes existing state).
- `--bitvmx-mode`: BitVMX identity source for coordinator (`docker` or `repo`). Default: `docker`.
- `--logs`: View logs from all 4 coordinators in real-time. Exits on Ctrl+C.
- `--kill`: Kill all existing running services and exit. Cannot be used with other options.
- `--start-mine`: Start background mining for both Anvil (every 1s) and Bitcoin regtest (every 5s). Runs until stopped.
- `--stop-mine`: Stop background mining processes started with `--start-mine`.

**Features:**
- Launches multiple services per client (block-indexer, log-indexer, user-api, coordinator)
- Automatic port configuration via `config/env_overrides/local-committee.env`
- Graceful shutdown handling with proper service teardown
- Panic recovery to ensure all services are properly stopped

## `cli-operations.sh` - Operations Toolkit

Handles operator operations and user operations across different environments (local, local-docker, regtest,
alphanet, testnet).

### Environment Variables

The following environment variables can be set to simplify multi-host deployments:

- **`UC_ENV`**: Sets the default environment (`local`, `local-docker`, `regtest`, `alphanet`, or `testnet`). Can be overridden with `--env` flag.
- **`UC_OPERATOR_ID`**: Sets the default operator ID (1-10) for `apply-stream` command. Can be overridden with `--operator-id` flag.
- **`UC_OPERATOR_ROLE`**: Sets the default operator role (`prover` or `verifier`) for `apply-stream` command. Can be overridden with `--role` flag.

All three must be exported in the shell before invoking the wrapper. If you use `direnv`, keeping them in `.envrc` is one way to do that.

**Example for multi-host deployment:**

```bash
# In .envrc (project root), set all UC_* variables
export UC_ENV="alphanet"
export UC_OPERATOR_ID=1
export UC_OPERATOR_ROLE="prover"

# Then you can run commands without specifying these flags each time
./cli-operations.sh operator apply-stream --stream-id 1
./cli-operations.sh operator fund
```

**Precedence:** command-line flags > exported environment variables.

### Usage Examples

```bash
./cli-operations.sh --help

# Operator: Fund operators on local-docker environment
./cli-operations.sh operator fund --env local-docker

# Operator: Fund operators and execute wallet commands automatically
./cli-operations.sh operator fund --env local-docker --execute

# Operator: Apply to stream (local auto-applies all 4 operators)
./cli-operations.sh operator apply-stream --stream-id 1

# Operator: Apply to stream on remote (requires operator-id and role)
./cli-operations.sh operator apply-stream --stream-id 1 --env alphanet --operator-id 1 --role prover

# Operator: Whitelist member addresses on CommitteeRegistry
./cli-operations.sh operator whitelist --contract-address 0x742d35... --env local

# User: Display user addresses and funding instructions
./cli-operations.sh user fund --env local

# User: Create pegin transaction
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --btc-pub-key 0x<32-byte-xonly-pubkey> --env local

# User: Create pegin transaction and execute wallet command automatically
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --btc-pub-key 0x<32-byte-xonly-pubkey> --env local-docker --execute

# User: Request pegout
./cli-operations.sh user pegout --value 1000000 --usr-pub-key 0x<33-byte-compressed-pubkey> --env local
```

### Command Structure

- **`operator`**: Operator management (`fund`, `whitelist`, `apply-stream`)
  - `fund`: Display bitcoin addresses and optionally execute wallet commands with `--execute`
  - `whitelist`: Whitelist member addresses on the `CommitteeRegistry` contract
- **`user`**: User operations (`fund`, `pegin`, `pegout`)
  - `fund`: Display user addresses and funding instructions
  - `pegin`: Display pegin command and optionally execute wallet command with `--execute`

### Supported Environments

- **`local`**: Local development setup (default)
- **`local-docker`**: Local docker deployment
- **`regtest`**: Remote regtest deployment
- **`alphanet`**: Remote alphanet deployment
- **`testnet`**: Remote testnet deployment

### Safety Features

- `--execute` is only supported for local environments (`local`, `local-docker`)
- Confirmation prompts for all remote operations (`regtest`, `alphanet`, `testnet`)
- Displays exact commands and HTTP requests before execution

## CLI Workspace Structure

The CLI tools are organized in a separate Cargo workspace under `cli/`:

```text
cli/
├── Cargo.toml          # CLI workspace configuration with shared dependencies
├── run/                # Local client launcher
│   ├── src/main.rs
│   └── Cargo.toml
└── operations/         # Operations toolkit
    ├── src/
    │   ├── main.rs
    │   ├── bitcoin_wallet.rs
    │   ├── rsk_wallet.rs
    │   ├── committee.rs
    │   ├── pegin.rs
    │   ├── pegout.rs
    │   ├── environments.rs
    │   ├── constants.rs
    │   └── utils.rs
    └── Cargo.toml
```

The CLI workspace is independent from the main Union Bridge workspace, allowing for faster compilation and easier maintenance of CLI-specific code.

## Usage Examples

### Local Development Setup

```bash
# 1. Bootstrap wallets, broker identities, and BitVMX runtime artifacts
./cli-setup-operators.sh --env local --ops 4

# 2. Fund operators (Bitcoin + Rootstock)
# Option A: Print commands to run manually
./cli-operations.sh operator fund

# Option B: Execute wallet commands automatically
./cli-operations.sh operator fund --execute

# 3. Run all 4 clients
./cli-run.sh --features anvil

# 4. Apply operators to stream
./cli-operations.sh operator apply-stream --stream-id 1
```

`cli-setup-operators.sh --env local` creates the local keystores consumed by `./cli-run.sh`.
Docker operator mode uses `docker.env` and container keystore paths instead of these cargo-mode keystore files.

### Regtest/Alphanet/Testnet Operations

```bash
# Fund operators (prints addresses to fund manually)
./cli-operations.sh operator fund --env alphanet

# Apply specific operator to stream
./cli-operations.sh operator apply-stream --stream-id 1 --env alphanet --operator-id 1 --role prover

# Create pegin transaction (prints command to run manually)
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --btc-pub-key 0x<32-byte-xonly-pubkey> --env alphanet

# Request pegout
./cli-operations.sh user pegout --value 1000000 --usr-pub-key 0x<33-byte-compressed-pubkey> --env alphanet
```

## Docker Integration

When using the `docker/operator` setup, you can use the `cli-operations.sh` tool to interact with dockerized operators:

```bash
# Fund operators running in docker
./cli-operations.sh operator fund --env local-docker

# Apply operators to stream
./cli-operations.sh operator apply-stream --stream-id 1 --env local-docker
```

See [docker/operator/README.md](../docker/operator/README.md) for more information on docker deployments.
