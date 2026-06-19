# Logging

This project uses the [`tracing`](https://docs.rs/tracing) ecosystem. A single
subscriber is built in [`common/runtime/src/logging.rs`](../crates/common/runtime/src/logging.rs)
(`CommonConfig::init_logger`) and reused by every service crate
(`block-indexer`, `log-indexer`, `coordinator`, `transaction-dispatcher`,
`user-api`).

Each service binary calls its own `Logger::init(log_dir)` early in `main`. The
returned `LogGuard` MUST be held for the lifetime of the process — dropping it
flushes and closes the background file-writer thread.

## Quick reference

| What you want                | How                                                |
|------------------------------|----------------------------------------------------|
| Change verbosity             | `RUST_LOG=...`                                     |
| Force JSON output            | `LOG_FORMAT=json`                                  |
| Force human-readable output  | `LOG_FORMAT=pretty`                                |
| Customize file location      | `--log-dir <DIR>` or `UB_LOG_DIR=<DIR>` (default: `./logs/`) |
| Per-operator log file name   | `CLIENT_ID=<N>` (set automatically by `scripts/run-clients.sh`)|
| Switch default format        | `ENVIRONMENT=local` (pretty) vs anything else (json) |

## Output format: pretty vs JSON

Selection precedence:

1. `LOG_FORMAT` — explicit override. Accepts `json` or anything else (treated as
   pretty, case-insensitive).
2. `ENVIRONMENT` — when `LOG_FORMAT` is unset:
   - `local` (or unset) → pretty
   - any other value (`docker`, `staging`, etc.) → JSON

### Pretty

Human-readable, ANSI-colored, single line per event with a bracketed level so
log scrapers can match `[ERROR]` / `[WARN]` / `[INFO]` / `[DEBUG]` / `[TRACE]`.
Default for local dev.

```
2026-05-19 10:14:22.041 [ INFO] [block_indexer::sync] caught up height=8421
```

When the multi-operator launcher (`scripts/run-clients.sh`) runs several services in
parallel, their stdout lines interleave; operator identity is disambiguated by
the per-operator log files (see [File output](#file-output)), not by a stdout
prefix.

### JSON

One JSON event per line, flattened, with the current span and the full span
ancestry included. Default in any non-`local` environment — intended for log
shippers / structured search.

```json
{"timestamp":"2026-05-19T10:14:22.041Z","level":"INFO","target":"block_indexer::sync","message":"caught up","height":8421,"span":{"name":"sync_loop"},"spans":[{"name":"sync_loop"}]}
```

`span` carries only the innermost (current) span; `spans` is the full array
from outermost to innermost. Filter on `span.<field>` for the immediate
context, or scan `spans[*].<field>` when you need to match an ancestor.

Notes:
- Pegin work in the coordinator runs inside a `pegin` span carrying the
  `pegin_id` of the pegin being processed. In JSON output every log emitted
  from inside `PeginFlow`/`PeginFlowProcessor` (and the `btc_signature`
  subflow when spawned from a pegin) includes `span.name="pegin"` and
  `span.pegin_id="<uuid>"`, so logs from parallel pegins can be filtered
  by `pegin_id`. In pretty output the same span context appears between the
  level and the target.
- Pegout work in the coordinator runs inside a `pegout` span carrying the
  `pegout_id` of the pegout being processed. In JSON output every log emitted
  from inside `PegoutFlow`/`PegoutFlowProcessor` (and the `btc_signature`
  subflow when spawned from a pegout) includes `span.name="pegout"` and
  `span.pegout_id="<uuid>"`, so logs from parallel pegouts can be filtered
  by `pegout_id`. In pretty output the same span context appears between the
  level and the target.
- Committee setup work in the coordinator runs inside a `committee_setup` span
  carrying the `committee_setup_id` of the in-flight flow (a stable UUID per
  setup attempt, distinct from the RSK `committeeId` which is only known after
  `NewCommitteePending`). In JSON output every log emitted from inside
  `SetupCommitteeFlow`/`SetupCommitteeProcessor` includes
  `span.name="committee_setup"` and `span.committee_setup_id="<uuid>"`, so the
  many BitVMX/RSK calls of a single committee formation can be filtered by
  `committee_setup_id`. In pretty output the same span context appears between
  the level and the target.
- File output is always written **without** ANSI codes, regardless of format.
- In JSON the operator identity should be carried via `CLIENT_ID` in your
  shipping pipeline.

## Log level / filtering modules

Module-level filtering uses the standard `tracing-subscriber` `EnvFilter`
syntax, read from `RUST_LOG`.

When `RUST_LOG` is **unset or empty**, the built-in default
(`DEFAULT_FILTER` in `crates/common/runtime/src/logging.rs`) is applied:

```
debug,
tarpc=warn,
alloy_provider=warn,alloy_pubsub=warn,alloy_rpc_client=warn,alloy_json_rpc=warn,
hyper=warn,hyper_util=warn,h2=warn,
reqwest=warn,rustls=warn,tower_http=warn,tungstenite=warn
```

i.e. `debug` for our own code, `warn` for the noisy third-party crates.

### Examples

```bash
# Bump everything to trace
RUST_LOG=trace ./scripts/run-clients.sh

# Only debug for our crates, keep dependencies at info
RUST_LOG=info,block_indexer=debug,coordinator=debug ./scripts/run-clients.sh

# Silence a specific module
RUST_LOG=debug,alloy_provider=off ./scripts/run-clients.sh

# Re-enable a noisy third-party crate temporarily (overrides DEFAULT_FILTER)
RUST_LOG=debug,hyper=debug ./scripts/run-clients.sh
```

> Setting `RUST_LOG` **replaces** the default filter — it doesn't merge with it.
> If you want our defaults plus one tweak, copy the relevant pieces of
> `DEFAULT_FILTER` into your `RUST_LOG`.

> A **present-but-empty** `RUST_LOG` (e.g. `RUST_LOG=${RUST_LOG:-}` as injected by
> the docker-compose files) is treated the same as unset — it falls back to
> `DEFAULT_FILTER` rather than disabling all output. A non-empty but unparseable
> value also falls back to the default.

### Filter syntax recap

```
<level>                       # global level
<crate>=<level>               # one crate
<crate>::<module>=<level>     # nested module
target[span_name]=<level>     # span-scoped
off | error | warn | info | debug | trace
```

See <https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html>
for the full grammar.

### Bridging the `log` crate

`tracing_log::LogTracer` is installed automatically by `try_init()` via
`tracing-subscriber`'s default `tracing-log` feature, so dependencies still
using the classic `log` macros (`log::info!`, etc.) flow through the same
subscriber and obey the same `RUST_LOG` filter.

## File output

A log file is always written. The directory is resolved in this order:

1. `--log-dir <DIR>` CLI flag, then
2. `UB_LOG_DIR` environment variable, then
3. `./logs/` (relative to the process's current working directory).

File naming:

- With `CLIENT_ID` set (multi-operator launcher): `<crate_name>-<CLIENT_ID>.log`.
  Several operators share one per-execution directory (`logs/YYMMDD/HHMMSS/`), so
  the id keeps their files from colliding.
- Otherwise (e.g. Docker operators, each with its own log directory): a plain,
  predictable `<crate_name>.log`. It is opened in append mode, so it accumulates
  across restarts.

The file writer is non-blocking; the `LogGuard` returned from `init_logger`
owns the worker thread. Drop it on shutdown to flush.

## Docker: persisting logs to host files (opt-in)

By default the Docker stacks log **only to stdout** (visible via
`docker compose logs`); the per-crate file described above is still written, but
inside the container, so it is lost when the container is removed. To also
persist the files onto the host, layer the opt-in override that bind-mounts the
container's `/app/logs` onto a host directory. The base compose files are left
untouched, so without the override the behavior is exactly as before.

**Operators** (`docker/operator/`) — pass `--logs` to the launcher:

```bash
./start-operators.sh --logs up -d        # stdout + host files
./start-operators.sh up -d               # stdout only (unchanged default)
```

Files land in `${LOG_DIR}`, which `scripts/setup-operators.sh` sets per operator
to `~/.union_bridge/op_N/logs/` (default `./logs` when unset):

```
~/.union_bridge/op_1/logs/coordinator.log
~/.union_bridge/op_1/logs/block-indexer.log
~/.union_bridge/op_1/logs/log-indexer.log
~/.union_bridge/op_1/logs/user-api.log
~/.union_bridge/op_1/logs/bitvmx-client.log
```

Override the host directory with `LOG_DIR` (shell env, or the operator's
`docker-compose.env`):

```bash
LOG_DIR=/var/log/union/op_1 ./start-operators.sh --op 1 --logs up -d
```

**bitvmx (standalone `rust-bitvmx-client`)** — add the override file:

```bash
docker compose -f docker-compose.yml -f docker-compose.logs.yml up -d
# → ./volumes/logs/bitvmx-client-<op>.log
```

Mechanics and caveats:

- The override file is `docker-compose.logs.yml` (present in both repos). It only
  adds the `/app/logs` bind mount; the union-client services already write their
  tracing file there, so nothing else changes for them.
- `bitvmx-client` logs only to stdout, so the override wraps `run.sh` with `tee`
  to write the file while still emitting to stdout. That file uses `tee -a`
  (appended across restarts, **not** rotated), and `docker stop` may wait for the
  stop grace period because PID 1 becomes the wrapping shell.

## Environment variables — summary

| Variable      | Purpose                                                | Default          |
|---------------|--------------------------------------------------------|------------------|
| `RUST_LOG`    | `EnvFilter` directives — overall and per-module level  | `DEFAULT_FILTER` |
| `LOG_FORMAT`  | `json` or `pretty` — overrides environment-based default | (unset)        |
| `ENVIRONMENT` | Selects default format: `local` → pretty, else → JSON  | `local`          |
| `UB_LOG_DIR`  | Directory for the per-crate log file                   | `./logs/`        |
| `CLIENT_ID`   | Operator id; included in the per-operator log file name | (unset)         |

## Common recipes

**Local dev, more verbose (file written under `./logs/`):**
```bash
RUST_LOG=trace ./scripts/run-clients.sh
```

**Local dev, write JSON to a file for inspection:**
```bash
LOG_FORMAT=json UB_LOG_DIR=./logs/manual ./scripts/run-clients.sh
```

**Run a single service binary directly with a log file:**
```bash
cargo run -p block-indexer -- --log-dir ./logs --config local
```

**Hunt down a noisy dependency (find which target to silence):**
```bash
RUST_LOG=debug ./scripts/run-clients.sh 2>&1 | grep WARN | awk '{print $3}' | sort -u
# then add `<target>=warn` (or `=off`) to RUST_LOG
```
