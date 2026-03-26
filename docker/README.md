# Docker

This directory contains Docker configurations for different deployment scenarios of the Union Bridge Client.

| Folder | Purpose |
|--------|---------|
| `local-infra/` | Local dev dependencies (blockchains + BitVMX) for running Union Client with cargo |
| `bitvmx-client/` | Base BitVMX client compose and configuration files |
| `operator/` | Full operator deployment (BitVMX + Union Client in Docker) |
| `build/` | Union Client Docker build and image management |

## Pre-requisites

When pushing or pulling to a private GitHub container registry, Docker asks for a personal access token. You can generate one on GitHub using this [link](https://github.com/settings/tokens/new). 

Make sure you create the token with registry access. Set up the token by running:

```bash
export GITHUB_REGISTRY_TOKEN=<your-token>
echo "$GITHUB_REGISTRY_TOKEN" | docker login ghcr.io -u "user" --password-stdin
```

## Quick Reference

| Scenario | What to use |
|----------|-------------|
| Local dev (Union Client with cargo) | `local-infra/` for blockchains + BitVMX |
| Full local stack (all in Docker) | `operator/` with `--env local` |
| Alphanet deployment | `operator/` with `--env alphanet` |
| Building Union Client images | `build/` |

## Folders

### `local-infra/`

Local development dependencies. Use this when running Union Client locally with `cargo` and you need the supporting infrastructure (blockchains + BitVMX) in Docker.

**Scripts:**
- `start_blockchains.sh` - Starts bitcoind (regtest) + anvil + deploys contracts
- `start_bitvmx.sh` - Starts 4 BitVMX client instances

**Contracts version:** By default, `start_blockchains.sh` uses the contracts version from `Cargo.toml` (pulls from
registry; if the image digest changed, runs a fresh deploy automatically). Override with `--contracts-tag local-build`
to build from a local contracts checkout (no digest-based auto-fresh; use `--fresh` when local contracts change).

**Typical workflow:**

```bash
cd docker/local-infra

# Start blockchains (first time or fresh start)
./start_blockchains.sh --fresh up -d

# Generate local per-operator BitVMX config under ~/.union_bridge/op_N/bitvmx
cd ../operator
<project_root>/cli-setup-operators.sh --env local --ops 4

# Start 4 BitVMX clients
cd ../local-infra
./start_bitvmx.sh --fresh up -d

# Then run Union Client locally with cargo (from project root)
cd ../..
./cli-run.sh --id 1
```

**BitVMX client ports (for local Union Client connection):**
- op_1 → localhost:22222
- op_2 → localhost:33333
- op_3 → localhost:44444
- op_4 → localhost:55554

**Useful commands:**

```bash
# Check status
./start_blockchains.sh ps
./start_bitvmx.sh ps

# View logs
./start_blockchains.sh logs -f
./start_bitvmx.sh logs -f

# Stop
./start_blockchains.sh down
./start_bitvmx.sh down

# Stop and remove volumes (clean state)
./start_blockchains.sh down --volumes
./start_bitvmx.sh down --volumes
```

### `bitvmx-client/`

BitVMX client base configuration and docker-compose definitions. Contains:

- `docker-compose.yml` - Base BitVMX client service definition (extended by other composes)
- `config/` - BitVMX configuration files for different environments:
  - `local/` - Local development (regtest)
  - `alphanet/`, `testnet/`, `regtest/` - Templates with **placeholders** (`PLACEHOLDER_KEY_STORAGE_PASSWORD`, `PLACEHOLDER_WALLET_*`, Bitcoin RPC `url` like `http://<bitcoin-user>:<password>@<ip>:<port>`, `p2p.address` like `/ip4/<p2p-host>/tcp/<p2p-port>`, etc.). Replace with real values in your deployment (e.g. `cli-setup-operators.sh`, operator `.env`, or patched copies under `BASE_STORAGE_PATH`).
- `check_bitvmx_updates.sh` - Script to fetch and compare upstream BitVMX compose changes

**Checking for BitVMX updates:**

```bash
cd docker/bitvmx-client
./check_bitvmx_updates.sh              # Check against main branch
./check_bitvmx_updates.sh -r v0.1.3    # Check against specific tag
```

### `operator/`

Full operator deployment (BitVMX + Union Client in Docker). Use this for production-like deployments where everything runs in containers.

**Components per operator stack:**
- BitVMX client
- Union Client services: `user-api`, `block-indexer`, `log-indexer`, `coordinator`

**Scripts:**
- `start_operators.sh` - Main script to manage operator stacks
- `<project_root>/cli-setup-operators.sh` - Creates or reuses broker identities and generated operator env files

**Compose files:**
- `docker-compose.yml` - Base operator compose
- `docker-compose.all.yml` - Overlay for running all 4 operators on one host (local)
- `docker-compose.one.yml` - Overlay for running one operator per host (alphanet/testnet)

**Quick start (local):**

```bash
cd docker/operator

# Prepare host-side operator artifacts once
<project_root>/cli-setup-operators.sh --env local --ops 4

# Start all 4 operators
./start_operators.sh --env local up -d

# Stop
./start_operators.sh --env local down
```

See [operator/README.md](operator/README.md) for detailed usage including alphanet deployment.

### `build/`

Union Client Docker build and image management.

- `Dockerfile` - Production image build
- `Dockerfile_builder` - Builder image for CI/CD
- `docker-compose.yml` - Union Client service definitions
- `d-*.sh` - Helper scripts for building/pushing images

See [build/README.md](build/README.md) for detailed usage.

## Environment Files

The Docker setup uses two kinds of environment files:

- **Operator compose env:** [`docker/operator/.env.sample`](operator/.env.sample) is the template — copy it to `.env.local`, `.env.alphanet`, or `.env.testnet` (not committed). [`.env.regtest`](operator/.env.regtest) is committed as the default **shared regtest lab** profile; change URLs/passwords for your infrastructure.
- **Generated per-operator** files under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`, created by `<project_root>/cli-setup-operators.sh`

`docker.env` is only consumed by Docker operator runs (`start_operators.sh` / docker compose).  
Local cargo mode (`./cli-run.sh`) does not read this file.

## Troubleshooting

### Port conflicts

Ensure these ports are free before starting:

| Service | Ports |
|---------|-------|
| Bitcoind | 18443 |
| Anvil | 8545 |
| BitVMX P2P | 22222, 33333, 44444, 55554 |
| BitVMX broker | 61180-61183 |
| User API | 40001-40004 |

### BitVMX "Inconsistent blockchain state" error

This usually means BitVMX database is out of sync with Bitcoin. Run a fresh start:

```bash
cd docker/local-infra
./start_blockchains.sh --fresh up -d
./start_bitvmx.sh --fresh up -d
```

### Container won't start

Check logs for the specific container:

```bash
docker logs <container_name>
```

For coordinator issues, it may need BitVMX to be healthy first. Try restarting after BitVMX is up.
