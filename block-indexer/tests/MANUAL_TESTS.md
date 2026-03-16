# Block Indexer — Manual Test Runbook

These scenarios validate long-running block-indexer behavior that cannot be automated in CI.
They are designed to run against a **real RSK node** (mainnet or testnet). Anvil can be used
as a local fallback (with `anvil --block-time 1` and `--features anvil`), but note that Anvil
produces standard Ethereum headers, not RSK-format headers — so it does not exercise the
production RSK-specific parsing paths.

## Prerequisites

- A running RSK node (preferred) or Anvil instance
- The `block-indexer-runner` CLI tool (in `tests/block-indexer-runner`) — it handles config
loading, storage isolation, and initial block hash resolution automatically.
- Config values can also be overridden via environment variables with the `UB__` prefix
(e.g., `UB__PROVIDER__ROOTSTOCK__URL=ws://...`).
- When using Anvil instead of RSK, add `--features anvil` to all `cargo run` commands.

### Runner CLI

> **All `cargo run` commands below must be run from the repository root** (i.e. `union-bridge-client/`),
> since `block-indexer-runner` and `block-indexer-validator` are workspace members in the root `Cargo.toml`.

```bash
# Start 100 blocks behind current best, tag the run "soak"
cargo run -p block-indexer-runner -- -f 100 -t soak --provider-url $PROVIDER_URL

# Start from a specific block height
cargo run -p block-indexer-runner -- -b 1 -t genesis --provider-url $PROVIDER_URL

# Override cache size
cargo run -p block-indexer-runner -- -b 1 -t large-cache --cache-size 1000000 --provider-url $PROVIDER_URL

# Use an environment config overlay (config/environment/<env>.toml)
cargo run -p block-indexer-runner -- -f 100 -t soak -e manual-test
```

Storage is automatically isolated under `/tmp/manual-tests/<tag>/database`.

---

## Validation

The `block-indexer-validator` tool (in `tests/block-indexer-validator`) checks:

- **Best block** exists in the store
- **No leftover checkpoint** (backward sync completed)
- **Chain continuity** — walks from best block to the initial block, verifying parent hash linkage and canonical block presence at every height
- **Provider comparison** (if `--provider-url` given) — compares store best block against the live node

Storage-only validation (without provider comparison) is also possible by omitting `--provider-url`.

---

## Scenarios

Set these once before running any scenario — they carry through into tmux sessions:

```bash
# RSK node (preferred — exercises production RSK-specific parsing)
export PROVIDER_URL=ws://<RSK_NODE_HOST>:<RSK_NODE_WS_PORT>/websocket

# Or use Anvil as a local fallback (standard Ethereum headers, not RSK-format)
anvil --block-time 1 > /dev/null 2>&1 &
export PROVIDER_URL=ws://127.0.0.1:8545
```

Create log directories:

```bash
mkdir -p /tmp/manual-tests/{soak,genesis,large-cache}
```

> **Anvil users:** insert `--features anvil` after the package name in all `cargo run`
> commands below, e.g. `cargo run -p block-indexer-runner --features anvil -- ...`

### 1. Long-run subscribe (24-hour soak test)

**Goal:** Verify the indexer is stable over an extended subscription period — no crashes, memory leaks, or missed blocks.


| Step | Action                                                                           |
| ---- | -------------------------------------------------------------------------------- |
| 1    | Start in a tmux/screen session (runner resolves the initial block automatically) |
| 2    | Let it run for **24 hours**                                                      |
| 3    | Stop with `Ctrl+C`                                                               |


```bash
tmux new-session -d -s soak-test \
  "cargo run -p block-indexer-runner -- -f 100 -t soak \
     --provider-url $PROVIDER_URL 2>&1 | tee /tmp/manual-tests/soak/output.log"
tmux attach-session -t soak-test
# detach: Ctrl+b, d
# after 24h, reattach and Ctrl+C
```

**Validation:**

```bash
cargo run -p block-indexer-validator -- \
  --storage-path /tmp/manual-tests/soak/database/blocks \
  --provider-url $PROVIDER_URL
```

**Pass criteria:**

- No crashes, panics, or OOM after 24 hours
- Storage contains a continuous chain from initial block to latest
- No checkpoint remains
- Log file does not show increasing memory warnings or repeated reconnections

---

### 2. Long-run backward sync from genesis

**Goal:** Verify the indexer can sync the full chain from the genesis block.


| Step | Action                                                                      |
| ---- | --------------------------------------------------------------------------- |
| 1    | Start in a tmux/screen session (runner fetches genesis hash via `-b 1`)     |
| 2    | Let it run until backward sync completes (hours, depending on chain length) |
| 3    | Stop with `Ctrl+C`                                                          |


```bash
tmux new-session -d -s genesis-sync \
  "cargo run -p block-indexer-runner -- -b 1 -t genesis \
     --provider-url $PROVIDER_URL 2>&1 | tee /tmp/manual-tests/genesis/output.log"
```

**Validation:**

```bash
cargo run -p block-indexer-validator -- \
  --storage-path /tmp/manual-tests/genesis/database/blocks \
  --provider-url $PROVIDER_URL
```

**Pass criteria:**

- Backward sync completes without errors
- Storage contains the full chain from genesis to current best
- No gaps in storage

---

### 3. Large cache + long backward sync from genesis

**Goal:** Verify the indexer handles a very large cache combined with a full chain sync (memory stress test).


| Step | Action                                                                        |
| ---- | ----------------------------------------------------------------------------- |
| 1    | Start in a tmux/screen session (runner handles genesis hash + cache override) |
| 2    | Let it run until backward sync completes and subscription starts              |
| 3    | Let it subscribe for a while, then stop                                       |


```bash
tmux new-session -d -s large-cache-sync \
  "cargo run -p block-indexer-runner -- -b 1 -t large-cache --cache-size 1000000 \
     --provider-url $PROVIDER_URL 2>&1 | tee /tmp/manual-tests/large-cache/output.log"
```

**Validation:**

```bash
cargo run -p block-indexer-validator -- \
  --storage-path /tmp/manual-tests/large-cache/database/blocks \
  --provider-url $PROVIDER_URL
```

**Pass criteria:**

- Backward sync completes without OOM
- Subscription works after sync
- Storage contains a continuous chain with no gaps
- Memory usage stays within reasonable bounds

