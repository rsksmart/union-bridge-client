# Local Infra

Local development dependencies. Use this when running Union Client locally with `cargo` and you need the supporting infrastructure (blockchains + BitVMX) in Docker.

## Related Docs

- [../README.md](../README.md): Docker flow selection
- [../../CONTRIBUTING.md](../../CONTRIBUTING.md): contributor setup and local running order
- [../operator/README.md](../operator/README.md): operator Docker flow

## `cli-infra.sh`

If you want the simplest local entry point, use `./cli-infra.sh` from the repository root. It wraps the
`start_blockchains.sh`, `start_bitvmx.sh`, and mining flows.

```text
# Start all docker infra (blockchains + bitvmx) + mining
./cli-infra.sh --start [--fresh] [--contracts-tag TAG]

# Stop mining + all docker infra
./cli-infra.sh --stop

# Start blockchains docker containers only
./cli-infra.sh --start-blockchains [--fresh] [--contracts-tag TAG]

# Stop blockchains docker containers only
./cli-infra.sh --stop-blockchains

# Start bitvmx docker containers only
./cli-infra.sh --start-bitvmx [--fresh]

# Stop bitvmx docker containers only
./cli-infra.sh --stop-bitvmx

# Start background mining (anvil + bitcoin)
./cli-infra.sh --start-mine

# Stop background mining
./cli-infra.sh --stop-mine
```

This is the easiest path for the local mode where Union Client runs with `cargo` and Bitcoin, Anvil, and BitVMX run in
Docker.

## Scripts

- `start_blockchains.sh` - Starts bitcoind (regtest) + anvil + deploys contracts
- `start_bitvmx.sh` - Starts 4 BitVMX client instances

## Contracts Version

By default, `start_blockchains.sh` uses the contracts version from `Cargo.toml` (pulls from registry; if the image
digest changed, runs a fresh deploy automatically). Override with `--contracts-tag local-build` to build from a local
contracts checkout (no digest-based auto-fresh; use `--fresh` when local contracts change).

## Typical Workflow

```bash
cd docker/local-infra

# Start blockchains (first time or fresh start)
./start_blockchains.sh --fresh up -d

# Generate local per-operator BitVMX config under ~/.union_bridge/op_N/bitvmx
cd ../..
./cli-setup-operators.sh --env local --ops 4

# Start 4 BitVMX clients
cd docker/local-infra
./start_bitvmx.sh --fresh up -d

# Then run Union Client locally with cargo (from project root)
cd ../..
./cli-run.sh --id 1
```

## BitVMX Client Ports

- op_1 -> localhost:22222
- op_2 -> localhost:33333
- op_3 -> localhost:44444
- op_4 -> localhost:55554

## Useful Commands

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
