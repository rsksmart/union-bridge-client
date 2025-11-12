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
```

**Features:**
- Launches multiple services per client (block-indexer, log-indexer, user-api, coordinator)
- Automatic port configuration via `multiclient.env`
- Graceful shutdown handling with proper service teardown
- Panic recovery to ensure all services are properly stopped

## `cli-operations.sh` - Operations Toolkit

Handles setup, operator operations, and user operations across different environments (local, alphanet, testnet).

```bash
./cli-operations.sh --help

# Setup: Create Rootstock wallets for 4 operators
./cli-operations.sh setup create-rootstock-wallets

# Operator: Fund operators on local-docker environment
./cli-operations.sh operator fund --env local-docker

# Operator: Apply to stream (local auto-applies all 4 operators)
./cli-operations.sh operator apply-stream --stream-id 1

# Operator: Apply to stream on remote (requires operator-id and role)
./cli-operations.sh operator apply-stream --stream-id 1 --env alphanet --operator-id 1 --role prover

# User: Create pegin transaction
./cli-operations.sh user pegin --rsk-address 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb --value 1000000 --packet-number 0 --env local

# User: Request pegout
./cli-operations.sh user pegout --value 1000000 --env local
```

### Command Structure

- **`setup`**: Initial configuration (create wallets)
- **`operator`**: Operator management (fund, apply-stream)
- **`user`**: User operations (pegin, pegout)

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
./cli-operations.sh operator fund

# 3. Run all 4 clients
./cli-run.sh --features anvil

# 4. Apply operators to stream
./cli-operations.sh operator apply-stream --stream-id 1
```

### Alphanet/Testnet Operations

```bash
# Fund operators (prints addresses to fund manually)
./cli-operations.sh operator fund --env alphanet

# Apply specific operator to stream
./cli-operations.sh operator apply-stream --stream-id 1 --env alphanet --operator-id 1 --role prover

# Create pegin transaction
./cli-operations.sh user pegin --rsk-address 0x742d35... --value 1000000 --packet-number 0 --env alphanet

# Request pegout
./cli-operations.sh user pegout --value 1000000 --env alphanet
```

## Docker Integration

When using the docker-integrated setup, you can use the `cli-operations.sh` tool to interact with dockerized operators:

```bash
# Fund operators running in docker
./cli-operations.sh operator fund --env local-docker

# Apply operators to stream
./cli-operations.sh operator apply-stream --stream-id 1 --env local-docker
```

See [docker-integrated/README.md](../docker-integrated/README.md) for more information on docker deployments.

