# Local Infra

This doc owns the Docker-backed local infrastructure used when Union Bridge runs locally with `cargo`
or against the Docker-first `local-rskj` stack.

For the full startup order, shared env rules, and the recommended local workflow, start with the
[Local Setup Guide](../../docs/LOCAL_SETUP.md).

Environment note:

- `./scripts/run-infra.sh` reads `BASE_STORAGE_PATH` and any other exported variables from your current shell; it does not source `.envrc` itself.
- For the Docker-backed local setup, the generated BitVMX configs are patched from `BITCOIND_URL` (read from your shell or `.envrc`). The expected local Docker value is `http://foo:rpcpassword@host.docker.internal:18443`.

## Related Docs

- [Docker Guide](../README.md): Docker flow selection
- [Local Setup Guide](../../docs/LOCAL_SETUP.md): canonical local workflow
- [Operator Docker Runtime Guide](../operator/README.md): local operator Docker runtime

## `scripts/run-infra.sh`

Run this wrapper from the repository root when you want the quickest entry point for local blockchains, BitVMX, and
background mining.

```text
# Start all Docker infra (blockchains + bitvmx) + mining
./scripts/run-infra.sh --start [--fresh] [--contracts-tag TAG] [--pull-contracts]

# Stop mining + all Docker infra
./scripts/run-infra.sh --stop

# Start blockchains + background mining
./scripts/run-infra.sh --start-blockchains [--fresh] [--contracts-tag TAG] [--pull-contracts]

# Stop background mining + blockchains
./scripts/run-infra.sh --stop-blockchains

# Stop background mining only
./scripts/run-infra.sh --stop-mining

# Start BitVMX only
./scripts/run-infra.sh --start-bitvmx [--fresh]

# Stop BitVMX only
./scripts/run-infra.sh --stop-bitvmx

``` 

## Scripts

- `start-blockchains.sh`: starts bitcoind (regtest) + Anvil for `local-anvil`, or bitcoind + RSKj + powpeg-node for `local-rskj`
- `scripts/run-infra.sh --start-blockchains`: wraps blockchain startup, bootstraps the Bitcoin miner wallet with 101 blocks when needed, and then starts background mining
- `start-bitvmx.sh`: starts 4 BitVMX client instances

Mining is coupled to the blockchain lifecycle in this wrapper:

- `scripts/run-infra.sh --start`: starts blockchains, BitVMX, and background mining
- `scripts/run-infra.sh --start-blockchains`: starts blockchains, ensures `mainwallet` has mature regtest funds, and starts background mining
- `scripts/run-infra.sh --stop-blockchains`: stops background mining and blockchains
- `scripts/run-infra.sh --stop-mining`: stops background mining only; run this if mining gets stuck

## Contracts Version

By default, `start-blockchains.sh` uses the contracts version from `Cargo.toml` and resolves the matching predeployed
Anvil image from `PREDEPLOYED_ANVIL_IMAGE_BASE`. That image contains an Anvil state snapshot with the local contracts
already deployed, so `cli-infra` does not run a contract deployment container during startup.

For registry-style tags, the script uses the local Docker image when it already exists. If the image is missing locally,
the script pulls it from GHCR. Pass `--pull-contracts` when you explicitly want to refresh the image from GHCR even if a
local copy already exists.

Override with `--contracts-tag local-build` when you want to build a predeployed Anvil image from a local contracts
checkout instead of using the registry image. Contract changes require rebuilding this image.

### Building A Predeployed Anvil Image

The predeployed Anvil image is built from the contracts repository. During the
image build, Docker starts a temporary Anvil instance, deploys the contracts,
dumps the resulting state to `/opt/anvil/predeployed-state.json`, and packages
that snapshot into the final image.

Build the image from a sibling contracts checkout:

```bash
cd ../union-bridge-contracts

docker buildx build \
  --platform linux/amd64 \
  -t ghcr.io/rsksmart/union-bridge-contracts-anvil:<tag> \
  -f ../union-bridge-client/docker/local-infra/anvil/Dockerfile_predeployed \
  .
```

Example:

```bash
docker buildx build \
  --platform linux/amd64 \
  -t ghcr.io/rsksmart/union-bridge-contracts-anvil:v0.4.1-alpha-10-4-2-2m \
  -f ../union-bridge-client/docker/local-infra/anvil/Dockerfile_predeployed \
  .
```

Validate the local image:

```bash
docker run --rm -p 8545:8545 \
  ghcr.io/rsksmart/union-bridge-contracts-anvil:<tag>
```

In another terminal:

```bash
cast rpc eth_chainId --rpc-url http://127.0.0.1:8545
```

`./scripts/run-infra.sh --start --fresh` can use the local image tag directly. If the
selected image tag is not present locally, startup pulls it from GHCR and fails
if the tag is not published. Use `--pull-contracts` to force a GHCR refresh.

## local-rskj RSKj And PowPeg Versions

`local-rskj` uses official Docker Hub images:

- RSKj tags: <https://hub.docker.com/r/rsksmart/rskj/tags>
- powpeg-node tags: <https://hub.docker.com/r/rsksmart/powpeg-node/tags>

The contracts are deployed from the local `union-bridge-contracts` checkout resolved by `CONTRACTS_CONTEXT_PATH`
(`../../../../union-bridge-contracts` by default). Until the native-bridge local-regtest deploy changes land upstream, keep
that sibling checkout on `fedejinich/chore/local-regtest-native-bridge`; see the required sibling repository setup in
the [Local Setup Guide](../../docs/LOCAL_SETUP.md#required-sibling-repositories).

The default tested tags live in [`rskj/.env`](./rskj/.env):

```bash
RSKJ_TAG=VETIVER-9.0.1
POWPEG_TAG=VETIVER-9.0.0.0
```

For a one-off run, pass the tags to `scripts/run-infra.sh`:

```bash
PLATFORM=linux/arm64 ./scripts/run-infra.sh --env local-rskj \
  --start-blockchains --fresh \
  --rskj-tag VETIVER-9.0.1 \
  --powpeg-tag VETIVER-9.0.0.0
```

Use `--fresh` when changing RSKj or powpeg-node versions so old chain data does
not leak into the new run.

## Scope

The documented local infra flow is still a 4-client BitVMX setup:

- `op_1` -> `localhost:22222`
- `op_2` -> `localhost:33333`
- `op_3` -> `localhost:44444`
- `op_4` -> `localhost:55554`

If you need the full run sequence, do not rebuild it here. Use the [Local Setup Guide](../../docs/LOCAL_SETUP.md) and come
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

If you need to debug the lower-level scripts directly instead of using `./scripts/run-infra.sh`, the minimal sequence is:

```bash
cd docker/local-infra
./start-blockchains.sh --fresh up -d
cd ../..
./scripts/setup-operators.sh --ops 4
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
| Anvil / RSKj HTTP | 8545 |
| RSKj WS | 8546 |
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
