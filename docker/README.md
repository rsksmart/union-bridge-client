# Docker

This is the Docker router for this repository.

For the full local development procedure, start with the [Local Setup Guide](../docs/LOCAL_SETUP.md). Use the docs here only
for Docker-specific details.

## Related Docs

- [Local Setup Guide](../docs/LOCAL_SETUP.md): canonical local setup, shared env rules, and runtime-mode map
- [Local Infra Guide](local-infra/README.md): local blockchains + BitVMX in Docker
- [Operator Docker Runtime Guide](operator/README.md): local operator Docker runtime
- [Docker Build Guide](build/README.md): image build and registry operations

## Which Docker Flow

| Scenario | Read |
| --- | --- |
| Run Union Client locally with `cargo`, but keep blockchains and BitVMX in Docker | [Local Infra Guide](local-infra/README.md) |
| Run local operators in Docker | [Operator Docker Runtime Guide](operator/README.md) |
| Build or publish Union Client images | [Docker Build Guide](build/README.md) |
| Check or refresh the tracked BitVMX compose template | [`bitvmx-client/`](#bitvmx-client) below |

## Directory Guide

### `local-infra/`

Owns the Docker-backed local infra helpers used by the recommended contributor workflow.

### `bitvmx-client/`

Owns the tracked BitVMX Docker compose template and the local config template used by the Docker flows in this repo.

Key files:

- `docker-compose.yml`
- `config/local/`
- `check-bitvmx-updates.sh`

### `operator/`

Owns the local operator Docker runtime:

- `start-operators.sh`
- operator env-file handling
- compose variant selection
- local operator troubleshooting

### `build/`

Owns image builds, builder images, GHCR pulls, and pushes.

## Troubleshooting

- local blockchains or BitVMX issues: [Local Infra Guide](local-infra/README.md)
- operator runtime issues: [Operator Docker Runtime Guide](operator/README.md)
