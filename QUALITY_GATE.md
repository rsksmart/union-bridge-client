# Quality Gate

> Engineering standards (this file) define what reviewers check. For Rust coding patterns and codebase-specific
> guidance see [`AGENTS.md`](AGENTS.md). For developer setup, environment, and local workflow see
> [`CONTRIBUTING.md`](CONTRIBUTING.md). Build/CI checks, release artefact rules, and review-process details are
> tracked separately.

This gate is a living standard. Bullets may be added, refined, or retired over time; treat it as the current baseline,
not a permanent contract. The rules are not all CI-enforceable, but they are the basis for code review feedback.

**Status of items.** Bullets in the **Aiming for** subsections are forward-looking goals; everything else is the current
baseline expected of new code.

> **Disclaimer:** This repository does not currently meet all of these standards. Existing gaps reflect integration
> surfaces that were still evolving when the code landed. **New and updated code is reviewed against the standards
> going forward.**

## Table of contents

- [Scope and classification](#scope-and-classification)
- [Workspace and crate boundaries](#workspace-and-crate-boundaries)
- [Dependencies](#dependencies)
- [Error handling](#error-handling)
- [Observability](#observability)
- [Testing approach](#testing-approach)
- [Unsafe code policy](#unsafe-code-policy)
- [Concurrency](#concurrency)
- [Configuration and secrets](#configuration-and-secrets)
- [Protocol compatibility](#protocol-compatibility)
- [Domain-specific expectations](#domain-specific-expectations)

## Scope and classification

Every crate in this workspace is classified into one of two tiers:

**Production code** — held to the strict bar in the rest of this document:

- `common`
- `protocol-params`
- `op-funding`
- `check-fork` (and its sub-crates)
- `transaction-dispatcher`
- `coordinator`
- `block-indexer`
- `log-indexer`
- `key-manager`

**Non-production code** — operator and developer tooling, held to a relaxed bar:

- `cli/run` — local launcher for development
- `cli/operations` — operator command-line tool
- `cli/bitcoin-wallet` — wallet management CLI
- `user-api` — REST wrapper around the production layer

The relaxed bar means `clippy::pedantic` is allowed (not denied) in these crates. All other bars — format, sort,
`clippy::all` + `clippy::cargo` at deny, tests, coverage, supply chain, visibility lints — still apply.

Any crate not listed above as non-production is production by default.

## Workspace and crate boundaries

- Library crates expose a stable public surface. Internal-only items are `pub(crate)` or behind a private module.
- Binaries are thin: business logic lives in libraries, `main.rs` is wiring (parse config, build runtime, run).
- A new crate is introduced only when it has at least one of: independent versioning need, distinct dependency set,
  distinct ownership boundary, or reuse outside the workspace. Otherwise prefer a module.

## Dependencies

- A net-new external dependency needs a one-line justification in the PR description: what existing workspace
  crate or `std` capability was insufficient, and why this crate over alternatives.
- Prefer existing workspace dependencies and `std` over introducing a new external crate.
- `cargo audit` runs in CI. Crates with open advisories at medium severity or above are not introduced;
  existing usages migrate off them when a replacement lands or an explicit ignore is recorded with a tracking
  item.
- Git dependencies are pinned to an immutable reference (tag or commit hash), never a branch.

## Error handling

- **Typed errors are required where a caller branches on them.** If a `match` decides retry vs. fail, maps to an HTTP
  status, or routes to different logging, the error must be a `thiserror` enum with meaningful variants and `#[from]`
  for internal conversions. The expected shape is an enum whose variants drive caller behaviour (e.g. distinguishing
  fatal vs. transient failure for retry decisions, or mapping to HTTP status codes).
- **Elsewhere, `anyhow::Result` + `.context(...)` is the default.** This holds for both binary entry points and internal
  crates whose errors only propagate upward to be logged. Context strings must state *intent*, not restate the call ("
  loading config from {path}", not "config error"). A bare `?` chain with no context anywhere on the path is a defect.
- **Errors crossing a serialization boundary must use a stable, documented shape.** HTTP responses, broker payloads, and
  any cross-service message carry an explicit enum or code — never `format!("{:?}", err)`, never a free-form `String`
  field that happens to hold an error message. The boundary type is documented (variant list, JSON shape) and changes to
  it are treated as wire-format changes.
- **Panics are reserved for genuinely unrecoverable invariants.** Any `unwrap()`/`expect()` on a non-test path carries
  either an inline comment naming the invariant (`// INVARIANT: bytes.len() checked above`) or a descriptive `expect`
  message that names the invariant in place of a comment. Startup-time `.expect("failed to load X")` is acceptable: the
  message names what failed and there is no recovery path.
- **Don't reach for `thiserror` as decoration.** A typed enum that exists only to be `?`propagated to `main` and printed
  adds boilerplate without buying anything. If you can't name a caller that branches, a wire consumer that deserializes,
  or a metric that keys off the variant, `anyhow` is the right tool.

## Observability

- **Log levels.** `error` for actionable failures, `warn` for recovered anomalies, `info` for state transitions, `debug`
  for engineering diagnostics. `trace` is acceptable but should not be the default level in any deployed environment.
- **Sensitive data.** Keys, signatures, and other secrets are never logged, at any level.
- **Aiming for:**
    - **Logging.** Production builds emit structured JSON logs. New modules use `tracing`; older frameworks in
      unmigrated areas are tolerated until they're replaced.
    - **Spans.** Async functions in request- or flow-handling paths carry `#[instrument]` (from `tracing`) with the
      fields relevant to correlation (peg id, tx hash, block height, operator id, etc.).
    - **Metrics.** Per-service counters and histograms for: requests received, peg-flow stage transitions, broker
      messages in/out, RPC errors, retry counts. Conventional registry: `metrics` crate exposing a Prometheus endpoint.

## Testing approach

- **Unit tests** colocated with the code they cover (`#[cfg(test)] mod tests`).
- **Integration tests** in each crate's `tests/` directory, exercising the public surface end-to-end with `mockall`
  mocked external boundaries.

- **E2E tests evolve in sync** with protocol changes, taking the design document for the change as input.
- **Aiming for:** property-based tests for consensus-sensitive logic, fuzz targets for wire-format parsing, and
  benchmarks for performance-sensitive paths. Concrete per-area commitments live in
  [Domain-specific expectations](#domain-specific-expectations).

## Unsafe code policy

- `#![forbid(unsafe_code)]` at every crate root that does not require it.
- Where `unsafe` is unavoidable (FFI, platform APIs that are unsafe by design), each `unsafe` block carries a
  `// SAFETY:` comment explaining the invariant being upheld.
- Any new `unsafe` block in production code requires explicit reviewer sign-off on the PR that introduces it, with the
  justification recorded in the PR description.

## Concurrency

- **Add concurrency only when a dependency forces it, not as a performance lever.** The system is not throughput-bound,
  so synchronous single-threaded code is the default.
- **Async code uses `tokio` exclusively; do not mix runtimes.** Cross-runtime call sites such as
  `Handle::current().block_on(...)` stay confined to non-worker-thread callers, since `block_on` from inside a tokio
  worker thread panics.
- **Long-running tasks expose cancellation (either via `CancellationToken` or `select!` on a shutdown channel). No
  detached, unjoined long-running tasks in production paths, whether `tokio::spawn` or `std::thread::spawn`.** Spawns
  store their `JoinHandle` so the process waits for orderly exit rather than racing them.
- **Shared mutable state is owned by a single task and accessed via channels, or guarded by a `Mutex`/`RwLock`** (
  `tokio::sync` where state crosses `.await`, `std::sync` otherwise) with documented lock-ordering. Any nested-lock site
  carries a `// LOCK ORDER:` comment naming the acquisition order.

## Configuration and secrets

- All configurations are loaded at startup; no environment-variable lookups deep in business logic.

## Protocol compatibility

- Backwards-incompatible changes are called out explicitly in the PR description (and in the design document for the
  change, when one exists) and require explicit reviewer acknowledgement before merge.
- Database migrations are bundled with the backwards-incompatible change that requires them.

## Domain-specific expectations

- **Indexers** (`block-indexer`, `log-indexer`): catch-up correctness on restart, reorg handling, and idempotency are
  first-class invariants and must have explicit tests.
- **Force flags** (`/tmp/FORCE_*` or similar debug hooks): allowed only behind a compile-time feature flag that is not
  enabled in release builds, and the gating mechanism itself is reviewed on the PR that introduces it.
- **Bitcoin transaction construction**: deterministic given inputs; fuzzed against the `bitcoin` crate's parser.
- **Aiming for:**
    - Consensus-sensitive code (fork detection, signing, signature aggregation): property-based tests covering reorg
      depth, equivocation handling, and replay-protection invariants are expected, not optional.
    - Differential fuzzing of Bitcoin transaction construction against an established parser, with byte-for-byte
      determinism assertions across repeated builds from identical inputs.