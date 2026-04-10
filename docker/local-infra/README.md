# Local Infra

This doc owns the Docker-backed local infrastructure used when Union Bridge runs locally with `cargo`.

For the full startup order, shared env rules, and the recommended local workflow, start with the
[Contributing Guide](../../CONTRIBUTING.md).

## Related Docs

- [Docker Guide](../README.md): Docker flow selection
- [Contributing Guide](../../CONTRIBUTING.md): canonical local workflow
- [Operator Docker Runtime Guide](../operator/README.md): local operator Docker runtime

## `cli-infra.sh`

Run this wrapper from the repository root when you want the quickest entry point for local blockchains, BitVMX, and
background mining.

```text
# Start all Docker infra (blockchains + bitvmx) + mining
./cli-infra.sh --start [--fresh] [--contracts-tag TAG]

# Stop mining + all Docker infra
./cli-infra.sh --stop

# Start blockchains only
./cli-infra.sh --start-blockchains [--fresh] [--contracts-tag TAG]

# Stop blockchains only
./cli-infra.sh --stop-blockchains

# Start BitVMX only
./cli-infra.sh --start-bitvmx [--fresh]

# Stop BitVMX only
./cli-infra.sh --stop-bitvmx

# Start or stop background mining
./cli-infra.sh --start-mine
./cli-infra.sh --stop-mine
```

## Scripts

- `start-blockchains.sh`: starts bitcoind (regtest) + anvil + deploys contracts
- `start-bitvmx.sh`: starts 4 BitVMX client instances

## Contracts Version

By default, `start-blockchains.sh` uses the contracts version from `Cargo.toml`. Override with `--contracts-tag
local-build` when you want to use a local contracts checkout instead of the registry image.

## Scope

The documented local infra flow is still a 4-client BitVMX setup:

- `op_1` -> `localhost:22222`
- `op_2` -> `localhost:33333`
- `op_3` -> `localhost:44444`
- `op_4` -> `localhost:55554`

If you need the full run sequence, do not rebuild it here. Use the [Contributing Guide](../../CONTRIBUTING.md) and come
back to this doc for flags, ports, and troubleshooting.

## Useful Commands

Run these from `docker/local-infra/`:

```bash
./start-blockchains.sh ps
./start-bitvmx.sh ps

./start-blockchains.sh logs -f
./start-bitvmx.sh logs -f

./start-blockchains.sh down
./start-bitvmx.sh down

./start-blockchains.sh down --volumes
./start-bitvmx.sh down --volumes
```

## Direct Script Entry Points

If you need to debug the lower-level scripts directly instead of using `./cli-infra.sh`, the minimal sequence is:

```bash
cd docker/local-infra
./start-blockchains.sh --fresh up -d
cd ../..
./cli-setup-operators.sh --ops 4
cd docker/local-infra
./start-bitvmx.sh --fresh up -d
```

That direct path is useful for isolating whether the problem is in blockchain bootstrap, operator artifact generation,
or the BitVMX stack itself.

## Troubleshooting

### Port Conflicts

Ensure these ports are free before starting:

| Service | Ports |
| --- | --- |
| Bitcoind | 18443 |
| Anvil | 8545 |
| BitVMX P2P | 22222, 33333, 44444, 55554 |
| BitVMX broker | 61180-61183 |
| User API | 40001-40004 |

### BitVMX "Inconsistent blockchain state" Error

This usually means the BitVMX database is out of sync with Bitcoin. Run a fresh start:

```bash
cd docker/local-infra
./start-blockchains.sh --fresh up -d
./start-bitvmx.sh --fresh up -d
```

### Container Won't Start

Check logs for the failing container:

```bash
docker logs <container_name>
```

If coordinator-related services fail, verify that BitVMX is healthy first.
