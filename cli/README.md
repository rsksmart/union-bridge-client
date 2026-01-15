# Union Bridge CLI Tools

The project includes two CLI tools in the `cli/` workspace for managing local development and operator operations.

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
- `--logs`: View logs from all 4 coordinators in real-time. Exits on Ctrl+C.
- `--kill`: Kill all existing running services and exit. Cannot be used with other options.
- `--start-mine`: Start background mining for both Anvil (every 1s) and Bitcoin regtest (every 5s). Runs until stopped.
- `--stop-mine`: Stop background mining processes started with `--start-mine`.

**Features:**
- Launches multiple services per client (block-indexer, log-indexer, user-api, coordinator)
- Automatic port configuration via `multiclient.env`
- Graceful shutdown handling with proper service teardown
- Panic recovery to ensure all services are properly stopped

## `cli-operations.sh` - Operations Toolkit

Handles setup, operator operations, and user operations across different environments (local, alphanet, testnet).

### Environment Variables

The following environment variables can be set to simplify multi-host deployments:

- **`UC_ENV`**: Sets the default environment (`local`, `local-docker`, `alphanet`, or `testnet`). Set in `.envrc` at project root. Can be overridden with `--env` flag.
- **`UC_OPERATOR_ID`**: Sets the default operator ID (1-4) for `apply-stream` command. Set in environment-specific `.env.*` files (`docker/operator/.env.alphanet`, `.env.testnet`, or `.env.local`). Can be overridden with `--operator-id` flag.
- **`UC_OPERATOR_ROLE`**: Sets the default operator role (`prover` or `verifier`) for `apply-stream` command. Set in environment-specific `.env.*` files. Can be overridden with `--role` flag.

**Example for multi-host deployment:**

```bash
# In .envrc (project root), set the default environment
export UC_ENV=alphanet

# In docker/operator/.env.alphanet, set operator-specific values for this host
export UC_OPERATOR_ID=1
export UC_OPERATOR_ROLE=prover

# Then you can run commands without specifying these flags each time
./cli-operations.sh operator apply-stream --stream-id 1
./cli-operations.sh operator fund
```

**Why this approach?**
- `UC_ENV` is in `.envrc` because it's needed to determine which `.env.*` file to load
- `UC_OPERATOR_ID` and `UC_OPERATOR_ROLE` are in `.env.*` files because they're operator-specific (each host runs one operator in multi-host deployments)

### Usage Examples

```bash
./cli-operations.sh --help

# Setup: Create Rootstock wallets for 4 operators
./cli-operations.sh setup create-rootstock-wallets

# Operator: Fund operators on local-docker environment
./cli-operations.sh operator fund --env local-docker

# Operator: Fund operators and execute wallet commands automatically
./cli-operations.sh operator fund --env local-docker --execute

# Operator: Apply to stream (local auto-applies all 4 operators)
./cli-operations.sh operator apply-stream --stream-id 1

# Operator: Apply to stream on remote (requires operator-id and role)
./cli-operations.sh operator apply-stream --stream-id 1 --env alphanet --operator-id 1 --role prover

# User: Create pegin transaction
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --packet-number 0 --env local

# User: Create pegin transaction and execute wallet command automatically
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --packet-number 0 --env alphanet --execute

# User: Request pegout
./cli-operations.sh user pegout --value 1000000 --env local
```

### Command Structure

- **`setup`**: Initial configuration (create wallets)
- **`operator`**: Operator management (fund, apply-stream)
  - `fund`: Display bitcoin addresses and optionally execute wallet commands with `--execute`
- **`user`**: User operations (pegin, pegout)
  - `pegin`: Display pegin command and optionally execute wallet command with `--execute`

### Supported Environments

- **`local`**: Local development setup (default)
- **`local-docker`**: Local docker deployment
- **`alphanet`**: Remote alphanet deployment
- **`testnet`**: Remote testnet deployment

### Safety Features

- Confirmation prompts for all remote operations (alphanet/testnet)
- Displays exact commands and HTTP requests before execution

## CLI Workspace Structure

The CLI tools are organized in a separate Cargo workspace under `cli/`:

```
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
# 1. Create wallets for 4 operators
./cli-operations.sh setup create-rootstock-wallets

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

### Alphanet/Testnet Operations

```bash
# Fund operators (prints addresses to fund manually)
./cli-operations.sh operator fund --env alphanet

# Fund operators and execute wallet commands automatically
./cli-operations.sh operator fund --env alphanet --execute

# Apply specific operator to stream
./cli-operations.sh operator apply-stream --stream-id 1 --env alphanet --operator-id 1 --role prover

# Create pegin transaction (prints command to run manually)
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --packet-number 0 --env alphanet

# Create pegin transaction and execute wallet command automatically
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --packet-number 0 --env alphanet --execute

# Request pegout
./cli-operations.sh user pegout --value 1000000 --env alphanet
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

