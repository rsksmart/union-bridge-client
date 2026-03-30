# Operator Docker Runtime

This document covers the Docker runtime for local operators only.

## Related Docs

- [../README.md](../README.md): Docker flow selection
- [../build/README.md](../build/README.md): image build and registry operations
- [../local-infra/README.md](../local-infra/README.md): local blockchains + BitVMX
- [../../cli/README.md](../../cli/README.md): local and remote operations CLI

In the examples below, `<project_root>` means the root of this repository checkout.

## Scope

This repo owns:

- local multi-operator Docker runtime
- generated runtime artifacts under `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/`
- local `start-operators.sh` usage
- local funding, logs, and troubleshooting

Remote runtime environments such as alphanet and testnet are not covered here.

## Prerequisites

Before starting operators, export the values consumed by local setup:

```bash
export BITCOIND_URL=http://user:password@localhost:18443
export KEY_STORE_PASSWORD=<your-password>
export USER_BITCOIN_WIF=<your-user-wif>
```

`BITCOIND_URL` is patched into the generated BitVMX operator YAMLs.
`KEY_STORE_PASSWORD` and `USER_BITCOIN_WIF` are stored in each generated
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`.

## 1. Prepare Local Operator Artifacts

Bootstrap the local runtime artifacts once on that machine:

```bash
cd <project_root>
./cli-setup-operators.sh --ops 4
```

This creates or refreshes:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/<service>.pem`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/union-client/<service>.pubkey_hash`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/...`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/keystore/{member,user}`

The setup script still patches the generated local BitVMX YAMLs with the current `BITCOIND_URL`, key-store password,
and the required broker pubkey hashes. Those placeholders exist in the tracked local BitVMX templates and are required
for the local workflow to work.

## 2. Start Local Blockchains and BitVMX

Use the local infra helpers:

```bash
cd <project_root>
./cli-infra.sh --start --fresh
```

Or run the lower-level scripts directly:

```bash
cd docker/local-infra
./start_blockchains.sh --fresh up -d
cd ../..
./cli-setup-operators.sh --ops 4
cd docker/local-infra
./start_bitvmx.sh --fresh up -d
```

## 3. Start or Stop Local Operators

Use the local-only operator wrapper:

```bash
cd docker/operator

# Start 4 operators
bash start-operators.sh up -d

# Start a different count
bash start-operators.sh --ops 6 up -d

# Clean and start again
bash start-operators.sh --fresh up -d

# Logs / status / stop
bash start-operators.sh logs -f
bash start-operators.sh ps
bash start-operators.sh down
```

Use `docker` for this local runtime.

## Local Environment File

The Docker runtime uses:

- tracked static environment file: [`docker/operator/.env.local`](.env.local)
- generated per-operator runtime files: `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker.env`

`docker.env` is consumed only by Docker operator runs.
Local cargo mode (`./cli-run.sh`) does not read that file.

## BitVMX Template Checks

The tracked upstream BitVMX compose file lives in [`../bitvmx-client/`](../bitvmx-client).
If you need to compare it with upstream:

```bash
cd ../bitvmx-client
./check_bitvmx_updates.sh
./check_bitvmx_updates.sh -r <branch-or-tag>
```

## Troubleshooting

### Missing operator env file

If `start-operators.sh` reports a missing `docker.env`, rerun:

```bash
./cli-setup-operators.sh --ops 4
```

### Missing BitVMX config

If `.union_bridge/op_N/bitvmx/` is missing or stale, delete the operator directory under
`${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/` and rerun `cli-setup-operators.sh`.

### Docker runtime scope

This document covers only the local Docker operator flow.
