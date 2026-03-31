# Docker

This document is the Docker entry point for this repository.
It helps you choose the right Docker flow owned by this repository.

## Related Docs

- [../CONTRIBUTING.md](../CONTRIBUTING.md): shared contributor setup and configuration
- [operator/README.md](operator/README.md): local operator Docker runtime
- [build/README.md](build/README.md): image build and registry operations

## Which Docker Flow

| Scenario | Read |
| --- | --- |
| Run Union Client locally with `cargo`, but keep blockchains and BitVMX in Docker | [local-infra/README.md](local-infra/README.md) |
| Run local operators in Docker | [operator/README.md](operator/README.md) |
| Build or publish Union Client images | [build/README.md](build/README.md) |
| Check or refresh the BitVMX compose template | [`bitvmx-client/`](#bitvmx-client) below |

## Directory Guide

### `local-infra/`

Local development dependencies. Use this when running Union Client locally with `cargo` and you need the supporting
infrastructure (blockchains + BitVMX) in Docker.

### `bitvmx-client/`

This directory contains the tracked BitVMX Docker compose template and the local config template used by the Docker
flows in this repo.

Key files:

- `docker-compose.yml`: base BitVMX client service definition
- `config/local/`: tracked local BitVMX config template
- `check_bitvmx_updates.sh`: compares the tracked compose file with upstream

### `operator/`

This is the operator Docker runtime flow. It owns:

- local runtime setup
- env-file driven operator startup
- generated runtime artifacts under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`
- `start-operators.sh` usage
- local funding, logs, and troubleshooting

### `build/`

This directory owns image builds, builder images, GHCR pulls, and pushes.

## Environment Files

The Docker setup uses two kinds of environment files:

- tracked static environment file: [`docker/operator/docker-deploy.env`](operator/docker-deploy.env)
- optional external environment file passed to `start-operators.sh --env-file <path>`
- generated per-operator files under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/{docker-compose.env,docker-service.env}`, created by
  `<project_root>/cli-setup-operators.sh`

The operator compose override is derived from the effective operator count.
`--op <ID>` or `NUM_OPERATORS=1` selects the single-operator host-network variant; `NUM_OPERATORS=2-10` selects the shared multi-operator variant.

`docker-compose.env` and `docker-service.env` are only consumed by local Docker operator runs (`start-operators.sh` / docker compose).
Local cargo mode (`./cli-run.sh`) does not read those files.

## Troubleshooting

- For `local-infra` issues, use [local-infra/README.md](local-infra/README.md).
- For local operator-stack issues, use [operator/README.md](operator/README.md).
