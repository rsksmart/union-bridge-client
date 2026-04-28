# Operator Docker Runtime

This doc owns the local operator Docker runtime driven from this repository.

For the full local setup order, shared env semantics, and the recommended contributor path, start with the
[Contributing Guide](../../CONTRIBUTING.md). This doc only covers operator-specific runtime detail.

## Related Docs

- [Docker Guide](../README.md): Docker flow routing
- [Contributing Guide](../../CONTRIBUTING.md): canonical local workflow
- [Local Infra Guide](../local-infra/README.md): local blockchains + BitVMX
- [CLI Tools Guide](../../cli/README.md): CLI operations

## Scope

This repo owns:

- local multi-operator Docker runtime
- env-file driven operator startup
- generated runtime artifacts under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`
- `start-operators.sh` usage
- local funding, logs, and troubleshooting

## Prerequisites

```bash
# Shared env used by local setup
export BITCOIND_URL=http://user:password@localhost:18443
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>

# Prepare staged operator payloads from the repository root.
# Add -y to skip the removal confirmation for existing op_N folders.
./cli-setup-operators.sh --ops 4
```

The generated files include:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/broker/<service>.pem`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/broker/<service>.pubkey_hash`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/...`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker-compose.env`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker-service.env`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/keystore/{member,user}`

`docker-compose.env` includes `KEYSTORE_DIR=${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/keystore`.
`start-operators.sh` loads that file automatically, so local operator runs do not need you to export `KEYSTORE_DIR`
by hand.

The setup flow also patches the generated local BitVMX YAMLs with the current `BITCOIND_URL`, the keystore password,
and the required broker pubkey hashes. `KEY_STORE_PASSWORD` and `USER_BITCOIN_WIF` are written into each operator's
`docker-service.env`.

If selected operator folders already exist, `cli-setup-operators.sh` lists them and asks before removing them. Use
`./cli-setup-operators.sh --ops 4 -y` for non-interactive reset and setup.

The coordinator and user-api containers bind-mount
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/keystore/` as `/keystore`. The same keystores serve local
cargo mode. Containers do not generate replacement keys; `cli-setup-operators.sh` creates them ahead of time via the
`key-manager` crate.

## Local Startup

Use this mode when blockchains come from `docker/local-infra`, but the Union Bridge services and BitVMX run from the
operator Docker runtime in this directory.

```bash
# 1. Start the local blockchains and background mining
./cli-infra.sh --start-blockchains --fresh

# 2. Prepare operator runtime artifacts
./cli-setup-operators.sh --ops 4

# 3. Start the operator Docker runtime from docker/operator/
bash start-operators.sh --fresh up -d
```

Notes:

- This sequence starts the blockchains from `docker/local-infra` and the wrapper's background mining loop.
- `start-operators.sh` includes the `bitvmx-client` service in the operator compose stack.
- `bash start-operators.sh up -d` reuses the current operator containers and volumes.
- After the Docker operator runtime is up, you can use `bash tests/run-flows.sh --env docker --setup`,
  then `--committee`, then the user-flow modes.

For broader workflow context, go back to the [Contributing Guide](../../CONTRIBUTING.md) or the
[Local Infra Guide](../local-infra/README.md).

## `start-operators.sh`

Run this from `docker/operator/`:

```bash
bash start-operators.sh up -d
bash start-operators.sh --op 3 up -d
bash start-operators.sh --ops 6 up -d
bash start-operators.sh --fresh up -d
bash start-operators.sh --env-file /path/to/docker-deploy.env up -d

bash start-operators.sh logs -f
bash start-operators.sh ps
bash start-operators.sh down
```

`--fresh` removes Docker volumes and databases for the operator stack but does not rotate Rootstock keys, because the
keystores come from the host `op_N/union-client/keystore/` directory prepared by `cli-setup-operators.sh`.

Compose selection is derived from the effective operator count:

- `--op <ID>` -> `docker-compose.one.yml`
- `NUM_OPERATORS=1` -> `docker-compose.one.yml`
- `NUM_OPERATORS=2-10` -> `docker-compose.all.yml`

## Environment Files

The Docker operator runtime uses:

- tracked static environment file: [`docker-deploy.env`](docker-deploy.env)
- optional external env file passed with `--env-file`
- generated per-operator files:
  - `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker-compose.env`
  - `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker-service.env`

`docker-deploy.env` can override shared deployment paths such as `CONFIG_DIR` and `RESOURCES_DIR`. When unset, Docker
falls back to the tracked repo copies under `../../config` and `../../resources`.

`--op <ID>` selects the staged payload under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_<ID>/` and is the intended
entry point for one-operator-per-host deployments.

`docker-compose.env` and `docker-service.env` are Docker operator runtime artifacts only. They are not read by the
local cargo client launched with `./cli-run.sh`. The generated env files point Docker at the host keystore directory
under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/keystore/`. `start-operators.sh` expects those
keystore files to already exist and fails fast if they are missing. If you bypass `start-operators.sh` and run
`docker compose` manually, export `KEYSTORE_DIR` yourself first.

## Operator Count Boundary

`start-operators.sh` supports `1-10` operators, but the documented local infra BitVMX flow in this repository still
describes 4 local BitVMX instances. Treat `--ops 5-10` as an operator-runtime surface, not as proof that the full
all-in-one local infra flow is documented beyond 4.

## BitVMX Template Checks

The tracked BitVMX compose template lives in [`../bitvmx-client/`](../bitvmx-client). To compare it with upstream:

```bash
cd ../bitvmx-client
./check-bitvmx-updates.sh
./check-bitvmx-updates.sh -r <branch-or-tag>
```

## Troubleshooting

### Missing Operator Env File

If `start-operators.sh` reports a missing `docker-compose.env` or `docker-service.env`, rerun:

```bash
./cli-setup-operators.sh --ops 4
```

### Missing BitVMX Config

If `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/` is missing or stale, rerun `./cli-setup-operators.sh`.
The setup script removes the affected selected operator directory before recreating it.

### Compose Variants

- [`docker-compose.all.yml`](docker-compose.all.yml): shared multi-operator flow
- [`docker-compose.one.yml`](docker-compose.one.yml): single-operator-per-host flow with host-network-ready runtime
  artifacts
