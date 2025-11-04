# Docker-integrated: BitVMX + Union Bridge Client

With this setup you get:

- 4 independent operator stacks in parallel (`op_1`..`op_4`) to simulate a committee.
- Each stack includes: BitVMX client + Union Client services (`user-api`, `block-indexer`, `log-indexer`,
  `coordinator`).
- A shared Docker network for BitVMX P2P across stacks.

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
- Anytime you want to verify your `docker-integrated/bitvmx-client/docker-compose.yml` is aligned with upstream.

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
`docker-integrated/bitvmx-client/docker-compose.fetched.yml`, and prints a unified diff against your working
`docker-integrated/bitvmx-client/docker-compose.yml`. It is safe to re-run at any time.

### 2) Choose your environment

- Local uses `.env.local`
- Alphanet uses `.env.alphanet`

### Start local blockchains (LOCAL ONLY)

This repository now provides a dedicated script to manage the local blockchain stack (bitcoind + anvil + contracts
deploy):

- Script: `start_blockchains.sh`
- Scope: **LOCAL ONLY**. It manages the local dev stack. It does nothing for alphanet/testnet environments.
- Operators are started separately with `start_operators.sh`.
- Note: if running for the first time, use the `--fresh` flag, this will create the bitcoin wallet, see below.

Examples:

```bash
# Start local blockchains
bash start_blockchains.sh --env local up -d

# Fresh start: tear down (including volumes) and start again
# This also recreates the Bitcoin wallet and (re)deploys contracts automatically
bash start_blockchains.sh --env local --fresh up -d

# Clean blockchain state only (removes volumes/state)
bash start_blockchains.sh --env local down --volumes

# Stop local blockchains
bash start_blockchains.sh --env local down

# Inspect status / logs
bash start_blockchains.sh --env local ps
bash start_blockchains.sh --env local logs -f
```

About `--fresh`:

- What it does: runs a cleanup of the local blockchain stack (`docker compose down --volumes`) before your command.
- When used with 'up': after containers start, it will automatically create the Bitcoin wallet 'mainwallet' and redeploy
  contracts.

#### Notes on contracts deployment

The contracts deployment runs once via the deploy-contracts container and then tears down, it is normal. You can
inspect its output (eg. to check contract addresses) via Docker Desktop or running the following command:
`bash start_blockchains.sh --env local logs deploy-contracts`

If the contracts code changes (eg. new tag), you must rebuild the `deploy-contracts` image. You can do it with:

```bash
# Rebuild the deploy-contracts image and start
bash start_blockchains.sh --env local --new-contracts-version --fresh up -d
```

### 3) Start or stop operator stacks

A `MEMBER_BITCOIN_WIF` needs to be exported in the environment. It is the Bitcoin private key (WIF) of the member/operator (used by BitVMX operations).
The `bitcoin-wallet` wallet needs to be using this key when generating operator transactions. You can generate one via the `bitcoin-wallet` with `generate_address`.
See [bitcoin-wallet README](bitcoin-wallet/README.md) for more info.

Show script help:

```bash
bash start_operators.sh --help
```

#### 3.1) Start local/dev (local bitcoind + anvil) using published images:

Start a single operator (operator 1):

```bash
bash start_operators.sh --op one --env local up -d
```

Start all 4 operators:

```bash
bash start_operators.sh --op all --env local up -d
```

Or explicitly specify the tag:

```bash
bash start_operators.sh --op one --env local --tag latest-anvil up -d
bash start_operators.sh --op all --env local --tag latest-anvil up -d
```

#### 3.2) Start alphanet:

Start a single operator (operator 1):

```bash
bash start_operators.sh --op one --env alphanet --tag latest-alphanet up -d
```

Start all 4 operators:

```bash
bash start_operators.sh --op all --env alphanet --tag latest-alphanet up -d
```

#### 3.3) Fund operator accounts (Rootstock and BitVMX Bitcoin accounts)

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
inspecting UTXOs, etc.). See the [`bitcoin-wallet` README](../bitcoin-wallet/README.md) for how to start and use the
CLI.

Use the helper script below to collect addresses and get ready-to-run `bitcoin-wallet` CLI commands. You must pass
`--env`:

```bash
# local/dev (shows mine_utxo and mine_block steps)
bash operator_scripts/fund_operators_bitcoin.sh --env local

# alphanet (prints only send_to_address)
bash operator_scripts/fund_operators_bitcoin.sh --env alphanet
```

Stop and remove everything (example for local/dev):

```bash
# Stop single operator (operator 1)
bash start_operators.sh --op one --env local down --volumes

# Stop all operators
bash start_operators.sh --op all --env local down --volumes
```

### 4) Viewing logs per operator project

You can tail logs per operator project name (`op_1`..`op_4`) using either docker compose directly or the `start_operators.sh` script:

Using docker compose directly:

```bash
docker compose -p op_1 logs -f
docker compose -p op_2 logs -f
docker compose -p op_3 logs -f
docker compose -p op_4 logs -f
```

Using the start_operators.sh script:

```bash
bash start_operators.sh --op one --env local logs -f
bash start_operators.sh --op all --env local logs
```

### 5) Interacting with the user-api

**Local environment:** Each operator stack exposes a user-api on different ports:

- `op_1` -> http://localhost:40001
- `op_2` -> http://localhost:40002
- `op_3` -> http://localhost:40003
- `op_4` -> http://localhost:40004

**Alphanet environment:** Each host runs one operator, accessible at:

- http://localhost:40001 (or your host's IP/domain)

#### Applying operators to a stream

Use the `committee_setup.sh` script to apply operators to a stream:

**Local:** Apply all 4 operators (2 Provers, 2 Verifiers):

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

## Tags and images

Currently, there are two main tags for the Docker images used in this setup:

- `latest-anvil`: local/dev images aligned with anvil usage.
- `latest-alphanet`: alphanet images aligned with the Alphanet infra.

## Notes

- **`start_operators.sh` environment-specific behavior:**
  - **Local**: Runs all 4 operators on one host. The `--op` flag is not allowed.
    - Uses `CLIENT_OP` values: `op_1`, `op_2`, `op_3`, `op_4`
    - Config location: `bitvmx-client/config/local/client/config/op_X.yaml`
  - **Alphanet**: Runs one operator per host. The `--op <ID>` flag (1-4) is required for startup commands (up, restart, start, create) but not allowed for other commands (logs, ps, down, etc.).
    - Uses `CLIENT_OP` value: `testnet_op_X` (where X is the operator ID from `--op` flag)
    - Config location: `bitvmx-client/config/alphanet/client/config/testnet_op_X.yaml`
    - The operator ID determines which BitVMX configuration file is loaded
- **Funding scripts** (`fund_operators_bitcoin.sh` and `fund_operators_rootstock.sh`):
  - Only require `--env` flag
  - Automatically detect which operator(s) to query based on environment
  - **Local**: Query all 4 operators
  - **Alphanet**: Query the single operator on this host using the `docker-integrated` project name
- **Committee setup script** (`committee_setup.sh`):
  - Requires `--env` and `--stream-id` flags
  - **Local**: Applies all 4 operators (2 Provers, 2 Verifiers) to the stream
  - **Alphanet**: Requires `--role` flag (Prover or Verifier) and applies only the single operator on this host
- The script forwards standard docker compose arguments (e.g., up, down, logs, ps, -d, --force-recreate). However, build is explicitly forbidden; use published images from the registry by tag instead.
- The script intentionally forbids building from source (build args are blocked). It is designed to consume registry images by tag.

## Troubleshooting

### Coordinator container doesn't start

The `coordinator` depends on BitVMX client health, which is tricky to detect atm. If `coordinator` didn't start, re-run
the up command.

### Bitcoin Wallet issues

See the `bitcoin-wallet` [README](../bitcoin-wallet/README.md) for more info.

### Resource conflicts

- **Port conflicts**: ensure ports `40001–40004`, `61180–61183`, and `22222/33333/44444/55554` are free.
  export `BITVMX_P2P_HOST` addresses accordingly in `start_operators.sh`.
- **Healthchecks**: services wait for each other; if something is stuck, try bringing stacks down as mentioned above,
  re-check env files, and start again.

### BitVMX error logs

**In local**, if you see the error _Inconsistent blockchain state_ or alike, it usually means BitVMX database is not in
sync with the Bitcoin node. This is quite frequent locally.

You can run a fresh start of both local blockchains and operators with these steps:

Restart clean blockchains

```bash
bash start_blockchains.sh --env local --fresh up -d
```

Restart clean operators:

```bash
# Single operator (operator 1)
bash start_operators.sh --op one --env local --fresh up -d

# All operators
bash start_operators.sh --op all --env local --fresh up -d
```

And now you can start operators as explained above.
