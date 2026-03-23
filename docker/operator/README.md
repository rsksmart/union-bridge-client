# Operator: BitVMX + Union Bridge Client

This setup provides flexible operator deployment configurations:

- **Local environment**: Run multiple independent operator stacks in parallel (`op_1`..`op_N`) on a single host to simulate a committee, using a shared Docker bridge network for BitVMX P2P communication. The default is 4 operators; use `--ops` when you need more.
- **Alphanet environment**: Run a single operator per host, allowing distributed committee deployment across multiple machines, using host network mode for BitVMX P2P connectivity
- Each stack includes: BitVMX client + Union Client services (`user-api`, `block-indexer`, `log-indexer`, `coordinator`)

## Pre-requisites

When pushing or pulling to a private GitHub container registry, Docker asks for a personal access token. You can generate one on GitHub using this [link](https://github.com/settings/tokens/new). 
Make sure you create the token with registry access. You can set up the token by running the following command:

```bash
export GITHUB_REGISTRY_TOKEN=<your-token>
echo "$GITHUB_REGISTRY_TOKEN" | docker login ghcr.io -u "user" --password-stdin.
```
## How to run it

### 1) Check and fetch BitVMX compose (re-run as needed)

Use this script whenever you need to fetch the latest BitVMX compose from upstream and compare it with your working
file. Typical times to run it:

- After a new `FairgateLabs/docker-bitvmx` release to see what's changed.
- Anytime you want to verify your `docker/bitvmx-client/docker-compose.yml` is aligned with upstream.

Run the script:

```bash
./check_bitvmx_updates.sh
```

Optionally select a specific branch or tag of `FairgateLabs/docker-bitvmx`:

```bash
./check_bitvmx_updates.sh --ref <branch-or-tag>
# or
./check_bitvmx_updates.sh -r <branch-or-tag>
```

The script clones `FairgateLabs/docker-bitvmx` at the chosen ref, saves the fetched compose as
`docker/bitvmx-client/docker-compose.fetched.yml`, and prints a unified diff against your working
`docker/bitvmx-client/docker-compose.yml`. It is safe to re-run at any time.

### 2) Choose your environment

This setup supports four deployment environments:

- **Local** (`.env.local`): Development environment that runs multiple operators on a single host with local Bitcoin and RSK nodes (default: 4, configurable via `setup_operators.sh --ops` or `start_operators.sh --ops`)
- **Alphanet** (`.env.alphanet`): Production-like environment where each host runs a single operator, connecting to the Alphanet testnet
- **Testnet** (`.env.testnet`): Production-like environment where each host runs a single operator, connecting to the Bitcoin testnet
- **Regtest** (`.env.regtest`): All 4 operators on one host, connected to shared regtest infrastructure (powpeg + node21)

#### BitVMX Network Modes

The BitVMX client requires different Docker network configurations depending on the deployment environment:

**Local environment (Bridge Network)**:
- Uses a shared Docker bridge network (`bitvmx-network`) for P2P communication between operators
- All operators run on the same host and communicate through Docker's internal network
- Each operator binds to a unique P2P port on the Docker bridge
- This isolated network allows multiple BitVMX clients to communicate without exposing ports to the host

**Alphanet/Testnet environment (Host Network)**:
- Uses Docker's host network mode (`network_mode: host`)
- The BitVMX client binds P2P ports directly to the host's network interfaces
- Required because BitVMX advertises its P2P address to other operators, and must be reachable at the host's actual IP address
- In a distributed deployment, operators on different physical machines need to connect to each other using real network addresses, not Docker internal IPs

### 3) Start local blockchains (LOCAL ONLY)

Use the scripts in `docker/local-infra/` to manage the local blockchain stack (bitcoind + anvil + contracts deploy):

- Script: `docker/local-infra/start_blockchains.sh`
- Scope: **LOCAL ONLY**. It manages the local dev stack.
- Operators are started separately with `start_operators.sh`.
- Note: if running for the first time, use the `--fresh` flag, this will create the bitcoin wallet.

Examples:

```bash
cd docker/local-infra

# Start local blockchains
./start_blockchains.sh up -d

# Fresh start: tear down (including volumes) and start again
# This also recreates the Bitcoin wallet and (re)deploys contracts automatically
./start_blockchains.sh --fresh up -d

# Clean blockchain state only (removes volumes/state)
./start_blockchains.sh down --volumes

# Stop local blockchains
./start_blockchains.sh down

# Inspect status / logs
./start_blockchains.sh ps
./start_blockchains.sh logs -f
```

About `--fresh`:

- What it does: runs a cleanup of the local blockchain stack (`docker compose down --volumes`) before your command.
- When used with 'up': after containers start, it will automatically create the Bitcoin wallet 'mainwallet' and redeploy
  contracts.

#### Notes on contracts deployment

The contracts deployment runs once via the deploy-contracts container and then tears down, it is normal. You can
inspect its output (eg. to check contract addresses) via Docker Desktop or running the following command:
`./start_blockchains.sh logs deploy-contracts`

**Contracts version:** By default, `start_blockchains.sh` uses the contracts version from `Cargo.toml` (union-contracts
tag) and pulls the deploy-contracts image from the registry. To build from a local contracts checkout instead, use
`--contracts-tag local-build`.

If the contracts code changes (e.g. new tag) and you use `local-build`, run a clean deploy:

```bash
# Clean deploy with local contracts
./start_blockchains.sh --fresh --contracts-tag local-build up -d
```

### 4) Start or stop operator stacks

Before starting Union services in Docker, prepare the operator artifacts once on that machine:

```bash
cd docker/operator

# Local/regtest on one host
./setup_operators.sh --env local --ops 4

# One operator per host
./setup_operators.sh --env alphanet --op 1
```

This creates or reuses:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/broker/block-indexer.pem`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/broker/block-indexer.pubkey_hash`
- the same pair for `log-indexer`, `user-api`, and `coordinator`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/<environment>/...` copied from `docker/bitvmx-client/config/<environment>`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/docker/<environment>.env`

`setup_operators.sh` prompts for `USER_BITCOIN_WIF` only when an operator env file is missing that value and persists it there.
Running setup again is incremental: existing broker identities are reused, and existing operator env files are refreshed in place so updated tags or derived broker values are applied without re-prompting for stored WIFs.
The generated BitVMX config copy is refreshed from the tracked template on each run, and the operator's client YAML is patched so `components.l2.pubkey_hash` matches that operator's coordinator broker identity when that field exists in the template.

`local` and `local-docker` share the same host-side runtime root by default:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/...`

So local cargo flows and local Docker operator flows reuse the same per-operator broker identities and generated runtime artifacts unless you change `BASE_STORAGE_PATH`.

`start_operators.sh` reads those generated operator env files to:

- mount each Union service's own broker PEM into its container
- inject the explicit remote broker `pubkey_hash` values required by `coordinator` and `user-api`

The Docker flow no longer relies on a shared broker key in `/keystore`.
The remaining shared `keystore` volume between `coordinator` and `user-api` is transitional and is only for the existing user/member keystore files.

BitVMX runtime config is now generated under `.union_bridge/op_N/bitvmx/<environment>/` instead of patching the tracked repo files in place.

Why BitVMX config is generated but Union Client config is not:

- Union Client already supports runtime configuration overrides via `UB__...` environment variables, so operator-specific values such as broker identities, remote pubkey hashes, ports, and WIFs can be injected without copying the tracked `config/` tree.
- BitVMX does not have an equivalent operator-runtime override layer in this repo. Its operator-specific values live inside YAML files, so the clean boundary is to treat `docker/bitvmx-client/config/<environment>` as a template and generate a per-operator host copy under `.union_bridge/op_N/bitvmx/<environment>/`.
- This keeps tracked files static while making all operator-runtime state live under `.union_bridge/op_N/...`.

#### Environment Variables

The following environment variables can be exported in the shell to simplify multi-host deployments:

- **`UC_ENV`**: Sets the default environment (`alphanet`, `testnet`, `local`, `local-docker`, or `regtest`). Can be overridden with `--env` flag.
- **`UC_TAG`**: Sets the default Docker image tag. Override it via `start_operators.sh --tag` or the shell environment.
  - Defaults: `latest-alphanet` (alphanet), `latest-testnet` (testnet), `latest-anvil` (local), `latest-regtest` (regtest)
- **`UC_OPERATOR_ID`**: Sets the default operator ID (1-10). Can be overridden with `--op` flag.
- **`UC_OPERATOR_ROLE`**: Sets the default operator role (`prover` or `verifier`). Used by `cli-operations.sh`.

**Example for multi-host deployment:**

```bash
# Export default values in the shell
export UC_ENV=alphanet
export UC_TAG=latest-alphanet
export UC_OPERATOR_ID=1
export UC_OPERATOR_ROLE=prover

# Then you can run commands without specifying flags
bash start_operators.sh up -d
bash start_operators.sh logs -f
bash start_operators.sh down
```

**Note:** Command-line flags override exported shell values when provided. `UC_TAG` precedence for `start_operators.sh` is: `--tag` > exported `UC_TAG` > static `docker/operator/.env.<environment>`.

#### Required Environment Variables

A `USER_BITCOIN_WIF` is required for the generated operator env files because `user-api` uses it for user endpoints (pegin/pegout operations).
`setup_operators.sh` reuses an exported `USER_BITCOIN_WIF` when present; otherwise it prompts once when creating a new operator env file, then reuses the stored value on later runs.
You can generate one via the `bitcoin-wallet` with `generate_address`.
See [bitcoin-wallet README](../../cli/bitcoin-wallet/README.md) for more info.

Note: The `bitcoin-wallet` component separately uses `MEMBER_BITCOIN_WIF` for member/operator BitVMX operations, but this is not required for starting operators via `start_operators.sh`.

Show script help:

```bash
bash start_operators.sh --help
```

#### 4.1) Start local/dev (local bitcoind + anvil) using published images:

Start operators (no `--op` flag for local; use `--ops` on `setup_operators.sh` and `start_operators.sh` when you want something other than the default 4 operators):

```bash
bash start_operators.sh --env local up -d
```

If you want to change the image tag for a single run:

```bash
bash start_operators.sh --env local --tag latest-anvil up -d
```

#### 4.2) Start alphanet:

On alphanet, each host runs a single operator. You must specify which operator (1-10) using `--op <ID>`:

```bash
# Start operator 1 on this host
bash start_operators.sh --op 1 --env alphanet up -d

# Start operator 2 on this host
bash start_operators.sh --op 2 --env alphanet up -d

# And so on for operators 3 through 10...
```

If you want to change the image tag for a single run:

```bash
bash start_operators.sh --op 1 --env alphanet --tag latest-alphanet up -d
```

#### 4.3) Start testnet:

On testnet, each host runs a single operator. You must specify which operator (1-10) using `--op <ID>`:

```bash
# Start operator 1 on this host
bash start_operators.sh --op 1 --env testnet up -d

# Start operator 2 on this host
bash start_operators.sh --op 2 --env testnet up -d

# And so on for operators 3 and 4...
```

If you want to change the image tag for a single run:

```bash
bash start_operators.sh --op 1 --env testnet --tag latest-testnet up -d
```

#### 4.4) Fund operator accounts (Rootstock and BitVMX Bitcoin accounts)

After the stacks are up, you can fund the operators' accounts on both Rootstock and Bitcoin (BitVMX internal operator
accounts).

##### Fund Rootstock (RSK)

You must pass the environment via `--env`.

- `local`: actually sends funds on local Anvil via `cast`
- `alphanet` or others: only prints the addresses to fund

```bash
# local
bash operator_scripts/fund_operators_rootstock.sh --env local

# alphanet (prints addresses to fund)
bash operator_scripts/fund_operators_rootstock.sh --env alphanet
```

##### Fund BitVMX Bitcoin accounts

We use the `bitcoin-wallet` crate (CLI) included in this repository to interact with Bitcoin node (funding addresses,
inspecting UTXOs, etc.). See the [`bitcoin-wallet` README](../../cli/bitcoin-wallet/README.md) for how to start and use the
CLI.

Use the helper script below to collect addresses and get ready-to-run `bitcoin-wallet` CLI commands. You must pass
`--env`:

```bash
# local/dev (shows mine_utxo and mine_block steps)
bash operator_scripts/fund_operators_bitcoin.sh --env local

# alphanet (prints only send_to_address)
bash operator_scripts/fund_operators_bitcoin.sh --env alphanet
```

Stop and remove everything:

```bash
# Local: stop all operators (no --op flag)
bash start_operators.sh --env local down --volumes

# Regtest: stop all operators (no --op flag)
bash start_operators.sh --env regtest down --volumes

# Alphanet: stop the operator on this host (no --op flag for down command)
bash start_operators.sh --env alphanet down --volumes
```

Regtest fresh clean+up is supported:

```bash
bash start_operators.sh --env regtest --fresh up -d
```

`start_operators.sh --env regtest` now auto-syncs BitVMX `checkpoint_height` and `wallet.start_height`
to current Bitcoin height (with timestamped backups) before startup commands, preventing stale-height
failures after regtest resets.

For full regtest fresh orchestration (wallet funding, contracts deploy, config update, bridge authorization, operators):

```bash
cd ../../
./cli-infra.sh --start-regtest --fresh
```

`./cli-infra.sh --start-regtest --fresh` is remote-only and executes `/home/ubuntu/regtest-fresh/regtest_fresh.sh` on `union-bridge-use2-1`.
`REGTEST_FRESH_MODE=local` is unsupported. You can override remote script location with `REGTEST_FRESH_REMOTE_SCRIPT`.

### 5) Viewing logs per operator project

**Local environment:** You can tail logs per operator project name (`op_1`..`op_N`) using docker compose directly:

```bash
docker compose -p op_1 logs -f
docker compose -p op_2 logs -f
docker compose -p op_3 logs -f
docker compose -p op_4 logs -f
```

Add more `op_N` projects only if you started them with `--ops`.

Using the `start_operators.sh` script:

```bash
bash start_operators.sh --env local logs -f
```

**Alphanet/Testnet environment:** View logs for the single operator on this host:

```bash
docker compose -p union-operator logs -f
```

Using the `start_operators.sh` script:
```bash
bash start_operators.sh --env alphanet logs -f
bash start_operators.sh --env testnet logs -f
```

### 6) Interacting with the user-api

**Local environment:** Each operator stack exposes a user-api on different ports. With the default 4-operator setup:

- `op_1` -> http://localhost:40001
- `op_2` -> http://localhost:40002
- `op_3` -> http://localhost:40003
- `op_4` -> http://localhost:40004

If you start more operators with `--ops`, the pattern continues (`op_5` -> `:40005`, etc.).

**Alphanet/Testnet environment:** Each host runs one operator, accessible at:

- http://localhost:40001 (or your host's IP/domain)

#### Applying operators to a stream

Use the `committee_setup.sh` script to apply operators to a stream:

**Local:** Apply all started operators:

```bash
bash operator_scripts/committee_setup.sh --stream-id <STREAM_ID> --env local
```

**Alphanet:** Apply the single operator on this host with a specific role:

```bash
# As Prover
bash operator_scripts/committee_setup.sh --stream-id <STREAM_ID> --env alphanet --role Prover

# As Verifier
bash operator_scripts/committee_setup.sh --stream-id <STREAM_ID> --env alphanet --role Verifier
```

**Testnet:** Apply the single operator on this host with a specific role:

```bash
# As Prover
bash operator_scripts/committee_setup.sh --stream-id <STREAM_ID> --env testnet --role Prover

# As Verifier
bash operator_scripts/committee_setup.sh --stream-id <STREAM_ID> --env testnet --role Verifier
```

## Tags and images

Currently, there are three main tags for the Docker images used in this setup:

- `latest-anvil`: local/dev images aligned with anvil usage.
- `latest-alphanet`: alphanet images aligned with the Alphanet infra.
- `latest-testnet`: testnet images aligned with the Testnet infra.

## Troubleshooting

### Coordinator container doesn't start

The `coordinator` depends on BitVMX client health, which is tricky to detect atm. If `coordinator` didn't start, re-run
the up command.

### Bitcoin Wallet issues

See the `bitcoin-wallet` [README](../../cli/bitcoin-wallet/README.md) for more info.

### Resource conflicts

- **Port conflicts**: ensure the ports for the operators you plan to run are free. With the default 4-operator local setup, that includes `40001–40004`, `61180–61183`, and `22222/33333/44444/55554`.
- **Healthchecks**: services wait for each other; if something is stuck, try bringing stacks down as mentioned above,
  re-check env files, and start again.

### BitVMX error logs

**In local**, if you see the error _Inconsistent blockchain state_ or alike, it usually means BitVMX database is not in
sync with the Bitcoin node. This is quite frequent locally.

You can run a fresh start of both local blockchains and operators with these steps:

Restart clean blockchains

```bash
cd docker/local-infra
./start_blockchains.sh --fresh up -d
```

Restart clean operators:

```bash
# Local: restart all operators (no --op flag)
bash start_operators.sh --env local --fresh up -d

# Alphanet: restart the operator on this host (requires --op for startup)
bash start_operators.sh --op 1 --env alphanet --fresh up -d

# Testnet: restart the operator on this host (requires --op for startup)
bash start_operators.sh --op 1 --env testnet --fresh up -d
```
