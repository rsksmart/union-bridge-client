# Docker

This document is the Docker entry point for the repository. It helps you choose the right Docker subdirectory; the
implementation-specific commands live in the README closest to the scripts that run them.

## Related Docs

- [../CONTRIBUTING.md](../CONTRIBUTING.md): shared contributor setup and configuration
- [operator/README.md](operator/README.md): full operator runtime flow
- [build/README.md](build/README.md): image build and registry operations

## Which Docker Flow

| Scenario | Read |
| --- | --- |
| Run Union Client locally with `cargo`, but keep blockchains and BitVMX in Docker | [local-infra/README.md](local-infra/README.md) |
| Run full operators in Docker | [operator/README.md](operator/README.md) |
| Build or publish Union Client images | [build/README.md](build/README.md) |
| Check or refresh the BitVMX compose template | [`bitvmx-client/`](#bitvmx-client) below |

## Directory Guide

### `local-infra/`

Local development dependencies. Use this when running Union Client locally with `cargo` and you need the supporting
infrastructure (blockchains + BitVMX) in Docker.

Read [local-infra/README.md](local-infra/README.md) for:

- `./cli-infra.sh` as the simplest local entry point
- `start_blockchains.sh`
- `start_bitvmx.sh`
- local cargo workflow details
- contracts image selection via `--contracts-tag local-build`
- local-infra troubleshooting

### `bitvmx-client/`

This directory contains the tracked BitVMX Docker compose template and environment-specific config templates used by
the Docker flows.

Key files:

- `docker-compose.yml`: base BitVMX client service definition
- `config/`: tracked BitVMX config templates by environment
- `check_bitvmx_updates.sh`: compares the tracked compose file with upstream

Example:

```bash
cd docker/bitvmx-client
./check_bitvmx_updates.sh
./check_bitvmx_updates.sh -r <branch-or-tag>
```

### `operator/`

This is the detailed Docker runtime doc for full operator stacks. It owns:

- environment selection
- generated runtime artifacts under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`
- `start_operators.sh` usage
- funding, logs, user API access, and troubleshooting

Read [operator/README.md](operator/README.md) for the actual commands.

### `build/`

This directory owns image builds, builder images, GHCR pulls, and pushes.

Read [build/README.md](build/README.md) for the actual commands.

## Environment Files

The Docker setup uses two kinds of environment files:

- tracked static environment files such as [`docker/operator/.env.local`](operator/.env.local),
  [`docker/operator/.env.alphanet`](operator/.env.alphanet), [`docker/operator/.env.regtest`](operator/.env.regtest),
  and [`docker/operator/.env.testnet`](operator/.env.testnet)
- generated per-operator runtime files under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`, created by
  `<project_root>/cli-setup-operators.sh`

`docker.env` is only consumed by Docker operator runs (`start_operators.sh` / docker compose).  
Local cargo mode (`./cli-run.sh`) does not read this file.

The full runtime-artifact layout is documented in [operator/README.md](operator/README.md).

## Troubleshooting

- For `local-infra` issues, use [local-infra/README.md](local-infra/README.md).
- For full operator-stack issues, use [operator/README.md](operator/README.md).
