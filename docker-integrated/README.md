# Docker-integrated: BitVMX + Union Bridge Client

With this setup you get:

- 4 independent operator stacks in parallel (`op_1`..`op_4`) to simulate a committee.
- Each stack includes: BitVMX client + Union Client services (`user-api`, `block-indexer`, `log-indexer`,
  `coordinator`).
- A shared Docker network for BitVMX P2P across stacks.

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
bash start_blockchains.sh --env local --new-contracts-version up -d
```

### 3) Start or stop the 4 operator stacks

A `WALLET_PRIVATE_KEY` needs to be exported in the environment. It is the Bitcoin private key (WIF) of the end user (used by `user-api`).
The `bitcoin-wallet` wallet needs to be using this key when generating the pegin transaction.

Show script help:

```bash
bash start_operators.sh --help
```

#### 3.1) Start local/dev (local bitcoind + anvil) using published images:

```bash
bash start_operators.sh --env local up -d
```

or explicitly specify the tag:

```bash
bash start_operators.sh --env local --tag latest-anvil up -d
```

#### 3.2) Start alphanet:

```bash
bash start_operators.sh --env alphanet --tag latest-alphanet up -d
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
bash start_operators.sh --env local down --volumes
```

### 4) Viewing logs per operator project

You can tail logs per operator project name (`op_1`..`op_4`):

```bash
docker compose -p op_1 logs -f
docker compose -p op_2 logs -f
docker compose -p op_3 logs -f
docker compose -p op_4 logs -f
```

### 5) Interacting with the user-api

Each stack exposes a user-api:

- `op_1` -> http://localhost:40001
- `op_2` -> http://localhost:40002
- `op_3` -> http://localhost:40003
- `op_4` -> http://localhost:40004

Example: apply 4 operators to a stream (Provers ×2, Verifiers ×2):

```bash
bash operator_scripts/committee_setup.sh --stream-id <STREAM_ID>
```

The script issues POSTs to `/apply-stream` on each user-api port.

## Tags and images

Currently, there are two main tags for the Docker images used in this setup:

- `latest-anvil`: local/dev images aligned with anvil usage.
- `latest-alphanet`: alphanet images aligned with the Alphanet infra.

## Notes

- `start_operators.sh` forwards standard docker compose arguments to docker compose (e.g., up, down, logs, ps, -d,
  --force-recreate). However, build is explicitly forbidden; use published images from the registry by tag instead.
- The script intentionally forbids building from source (build args are blocked). It is designed to consume registry
  images by tag.
- It will create the external Docker network `bitvmx-shared-network` (`172.20.0.0/16`) if it doesn't exist.

## Troubleshooting

### Coordinator container doesn't start

The `coordinator` depends on BitVMX client health, which is tricky to detect atm. If `coordinator` didn't start, re-run
the up command.

### Bitcoin Wallet issues

See the `bitcoin-wallet` [README](../bitcoin-wallet/README.md) for more info.

### Resource conflicts

- **Port conflicts**: ensure ports `40001–40004`, `61180–61183`, and `22222/33333/44444/55554` are free.
- **Network conflict**: if `172.20.0.0/16` is in use, recreate the `bitvmx-shared-network` with a different subnet and
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

Restart clean operators

```bash
bash start_operators.sh --env local --fresh up -d
```

And now you can start operators as explained above.