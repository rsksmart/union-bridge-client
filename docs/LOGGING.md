# Logging

This project uses the [`tracing`](https://docs.rs/tracing) ecosystem. A single
subscriber is built in [`common/src/config.rs`](../common/src/config.rs)
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
| Per-operator log file name   | `CLIENT_ID=<N>` (set automatically by `cli-run.sh`)|
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

When the multi-operator launcher (`cli-run.sh`) runs several services in
parallel, their stdout lines interleave; operator identity is disambiguated by
the per-operator log files (see [File output](#file-output)), not by a stdout
prefix.

### JSON

One JSON event per line, flattened, with the current span included. Default in
any non-`local` environment — intended for log shippers / structured search.

```json
{"timestamp":"2026-05-19T10:14:22.041Z","level":"INFO","target":"block_indexer::sync","message":"caught up","height":8421,"span":{"name":"sync_loop"}}
```

Notes:
- Pegout work in the coordinator runs inside a `pegout` span carrying the
  `pegout_id` of the pegout being processed. In JSON output every log emitted
  from inside `PegoutFlow`/`PegoutFlowProcessor` (and the `btc_signature`
  subflow when spawned from a pegout) includes `span.name="pegout"` and
  `span.pegout_id="<uuid>"`, so logs from parallel pegouts can be filtered
  by `pegout_id`. In pretty output the same span context appears between the
  level and the target.
- File output is always written **without** ANSI codes, regardless of format.
- In JSON the operator identity should be carried via `CLIENT_ID` in your
  shipping pipeline.

## Log level / filtering modules

Module-level filtering uses the standard `tracing-subscriber` `EnvFilter`
syntax, read from `RUST_LOG`.

When `RUST_LOG` is **unset**, the built-in default
(`DEFAULT_FILTER` in `common/src/config.rs`) is applied:

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
RUST_LOG=trace ./cli-run.sh

# Only debug for our crates, keep dependencies at info
RUST_LOG=info,block_indexer=debug,coordinator=debug ./cli-run.sh

# Silence a specific module
RUST_LOG=debug,alloy_provider=off ./cli-run.sh

# Re-enable a noisy third-party crate temporarily (overrides DEFAULT_FILTER)
RUST_LOG=debug,hyper=debug ./cli-run.sh
```

> Setting `RUST_LOG` **replaces** the default filter — it doesn't merge with it.
> If you want our defaults plus one tweak, copy the relevant pieces of
> `DEFAULT_FILTER` into your `RUST_LOG`.

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

- With `CLIENT_ID` set (multi-operator launcher):
  `<crate_name>-<CLIENT_ID>.log` — stable name per operator, since the parent
  directory is already per-execution (`logs/YYMMDD/HHMMSS/`).
- Otherwise: `<crate_name>-<YYYYMMDD_HHMMSS>.log`.

The file writer is non-blocking; the `LogGuard` returned from `init_logger`
owns the worker thread. Drop it on shutdown to flush.

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
RUST_LOG=trace ./cli-run.sh
```

**Local dev, write JSON to a file for inspection:**
```bash
LOG_FORMAT=json UB_LOG_DIR=./logs/manual ./cli-run.sh
```

**Run a single service binary directly with a log file:**
```bash
cargo run -p block-indexer -- --log-dir ./logs --config local
```

**Hunt down a noisy dependency (find which target to silence):**
```bash
RUST_LOG=debug ./cli-run.sh 2>&1 | grep WARN | awk '{print $3}' | sort -u
# then add `<target>=warn` (or `=off`) to RUST_LOG
```
