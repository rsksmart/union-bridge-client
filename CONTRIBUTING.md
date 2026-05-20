# Contributing

> Engineering standards and team conventions for contributors to this repository. For Rust coding patterns and
> codebase-specific guidance see [`AGENTS.md`](AGENTS.md). For developer setup, environment, and local workflow
> see [`LOCAL_SETUP.md`](LOCAL_SETUP.md). Build/CI checks, release artefact rules, and review-process details
> are tracked separately.

This document is a living standard. Bullets may be added, refined, or retired over time; treat it as the current
baseline, not a permanent contract. The rules are not all CI-enforceable, but they are the basis for code review
feedback.

**Status of items.** Bullets in the **Aiming for** subsections are forward-looking goals; everything else is the current
baseline expected of new code.

> **Disclaimer:** This repository does not currently meet all of these standards. Existing gaps reflect integration
> surfaces that were still evolving when the code landed. **New and updated code is reviewed against the standards
> going forward.**

## Table of contents

- [Scope and classification](#scope-and-classification)
- [Build reproducibility](#build-reproducibility)
- [Format and lint](#format-and-lint)
- [Coverage](#coverage)
- [Documentation build](#documentation-build)
- [Static analysis](#static-analysis)
- [Workspace and crate boundaries](#workspace-and-crate-boundaries)
- [Dependencies](#dependencies)
- [Error handling](#error-handling)
- [Defensive coding](#defensive-coding)
- [Observability](#observability)
- [Testing approach](#testing-approach)
- [Unsafe code policy](#unsafe-code-policy)
- [Concurrency](#concurrency)
- [Configuration and secrets](#configuration-and-secrets)
- [Protocol compatibility](#protocol-compatibility)
- [Domain-specific expectations](#domain-specific-expectations)
- [Team conventions and hooks](#team-conventions-and-hooks)

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

## Build reproducibility

- The toolchain is pinned via `rust-toolchain.toml`. CI and contributor checkouts use the same channel and
  components.
- `Cargo.lock` is committed. CI builds and contributor verifications use `cargo … --locked`.
- The minimum supported Rust version is declared as `rust-version` in the workspace `Cargo.toml` and matches
  the channel pinned in `rust-toolchain.toml`.
- Crate `edition` is consistent across the workspace.
- Dockerfile base images are pinned by digest, not a floating tag.

## Format and lint

- `cargo fmt --all -- --check`, `cargo sort --workspace --check`, and
  `cargo clippy --workspace --all-targets --all-features --locked -D warnings` must pass on every PR, for every
  Cargo workspace in the repository.
- Workspace lint configuration in each `Cargo.toml` (`clippy::all`, `clippy::pedantic`, `clippy::cargo` at deny)
  is preserved. Loosening a group requires reviewer justification recorded in the relevant `Cargo.toml`.
- The helper scripts under [`.hooks/`](.hooks/) are the single source of truth for these commands; CI and local
  hooks both shell out to them, so the surfaces cannot drift.

## Coverage

- **Aiming for:**
    - Line coverage of **75%** workspace-wide for library crates, measured by `cargo-llvm-cov` on every PR and
      tag. Binary entrypoints (`*/src/main.rs`) are excluded from the measurement.
    - A drop greater than 5 percentage points relative to the previous tag triggers a reviewer question, not an
      automatic fail.

## Documentation build

- `cargo doc --workspace --no-deps --all-features` succeeds with `RUSTDOCFLAGS=-D warnings` on every PR. Dead
  doc links, stale references, and broken intra-doc paths are build failures, not warnings.

## Static analysis

- Semgrep runs in CI on every PR. New high-severity findings fail the build; false positives are flagged and
  discussed in the PR rather than silenced.
- CodeQL (Rust and GitHub Actions analysis) runs in CI on every PR and must pass before merge.

## Workspace and crate boundaries

- Library crates expose a stable public surface. Internal-only items are `pub(crate)` or behind a private module.
- Binaries are thin: business logic lives in libraries, `main.rs` is wiring (parse config, build runtime, run).
- A new crate is introduced only when it has at least one of: independent versioning need, distinct dependency set,
  distinct ownership boundary, or reuse outside the workspace. Otherwise prefer a module.

## Dependencies

- A net-new external dependency needs a one-line justification in the PR description: what existing workspace
  crate or `std` capability was insufficient, and why this crate over alternatives.
- Prefer existing workspace dependencies and `std` over introducing a new external crate.
- Feature flags enabled on a dependency are kept to the minimum the consuming crate actually needs. Enabling
  "defaults plus a couple just in case" is a defect; each enabled feature is reviewable in the PR diff and
  carries weight (more code surface, more transitive deps, more attack surface).
- `cargo audit` runs in CI. Crates with open advisories at medium severity or above are not introduced;
  existing usages migrate off them when a replacement lands or an explicit ignore is recorded with a tracking
  item.
- Git dependencies are pinned by commit hash, never a branch or tag.
- GitHub Actions in workflow files (`.github/workflows/*.yml`) are pinned by commit hash, with a version
  comment for human readability. Floating refs (`@v4`, `@main`) are not allowed; the pin guarantees the
  action's behaviour can't silently change between runs.
- Dependency sources are restricted to `crates.io` and the known git remotes (`github.com/rsksmart/*`,
  `github.com/FairgateLabs/*`). New remotes require an explicit reviewer note in the PR description.

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
- **Panics are reserved for genuinely unrecoverable invariants.** The panicking APIs — `panic!`, `unreachable!`,
  `todo!`, `assert!`, `assert_eq!`, `debug_assert!`, `unwrap()`, `expect()` — are never reached from untrusted
  input or across FFI boundaries. Any `unwrap()`/`expect()` on a non-test path carries either an inline comment
  naming the invariant (`// INVARIANT: bytes.len() checked above`) or a descriptive `expect` message that names
  the invariant in place of a comment. Startup-time `.expect("failed to load X")` is acceptable: the message
  names what failed and there is no recovery path.
- **Don't reach for `thiserror` as decoration.** A typed enum that exists only to be `?`propagated to `main` and printed
  adds boilerplate without buying anything. If you can't name a caller that branches, a wire consumer that deserializes,
  or a metric that keys off the variant, `anyhow` is the right tool.

## Defensive coding

These rules apply to any path that handles external or untrusted input — HTTP/RPC bodies, broker messages, on-chain
event payloads, file contents, CLI arguments. Trusted-internal paths can relax some of these, but relaxation is the
exception, not the default.

- **Arithmetic on untrusted values is checked.** Use `checked_*` for fallible math that should refuse on overflow,
  `saturating_*` for clamping, `try_into` for narrowing conversions. Bare `+ - * /` on external values is a defect.
  Truncating `as` casts (e.g., `usize as u32`, `i64 as u32`) require an inline justification or migration to
  `try_into`. Division and remainder operators check the divisor; never `a / b` where `b` could be zero from input.
  `wrapping_*` / `overflowing_*` are reserved for cases where wraparound is the algorithm (e.g., modular arithmetic
  in cryptography) and the choice is commented at the call site.
- **Resource consumption from external input is bounded.** Anything that accumulates from a stream — `read_to_end`,
  `read_to_string`, `collect::<Vec<_>>()`, `format!` over attacker-sized data, `Vec::reserve`/`with_capacity` from
  a length field — is bounded by `take(N)`, a streaming parser, or an explicit cap. Channels and queues are
  bounded. Recursive parsing uses a depth/iteration cap. HTTP / RPC / broker boundaries enforce request-size limits
  and timeouts at the edge; rejecting an oversized request early is preferred to letting it drain the system.
- **Indexing on untrusted offsets uses checked accessors.** `slice[i]` / `vec[i]` panic on out-of-bounds; use
  `get(i)` / `get_mut(i)` when the index comes from external input or is otherwise not provably in range. String
  slicing must respect UTF-8 boundaries — operate on `chars()` / `char_indices()` for user-visible text, not byte
  offsets. `*_unchecked` variants (`get_unchecked`, `from_utf8_unchecked`, `slice::from_raw_parts`) are `unsafe`
  and follow the [Unsafe code policy](#unsafe-code-policy) — never on attacker-controlled offsets.

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
- `cargo test --workspace --locked --all-features` must pass before merge. Tests must not require manual setup
  beyond what `tests/README.md` documents.
- `#[ignore]` is added only with an inline comment naming the reason, and a linked tracking item if the ignore
  is temporary.
- **Aiming for:** property-based tests for consensus-sensitive logic, fuzz targets for wire-format parsing, and
  benchmarks for performance-sensitive paths. Concrete per-area commitments live in
  [Domain-specific expectations](#domain-specific-expectations).

## Unsafe code policy

- `#![forbid(unsafe_code)]` at every crate root that does not require it.
- Where `unsafe` is unavoidable (foreign-function calls, platform APIs that are unsafe by design), each
  `unsafe` block carries a `// SAFETY:` comment explaining the invariant being upheld.
- Patterns that earn extra scrutiny inside an `unsafe` block: `std::mem::transmute`, raw-pointer dereferences
  and `std::ptr::*` reads/writes, `slice::from_raw_parts(_mut)`, `str::from_utf8_unchecked`,
  `get_unchecked(_mut)`, `MaybeUninit::assume_init`, `ManuallyDrop`, and `extern "C"` / `libc` FFI calls.
  These appear only in the smallest possible `unsafe` block, with inputs validated at the boundary above.
- Any new `unsafe` block in production code requires explicit reviewer sign-off on the PR that introduces it,
  regardless of how minor the rest of the diff is. The review focuses on the `// SAFETY:` comment and the invariant
  it claims; the justification is also recorded in the PR description.
- FFI surfaces and `unsafe` code paths are covered by fuzz targets or explicit edge-case tests when the input
  space is adversarial.

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

## Team conventions and hooks

This repository follows [Conventional Commits](https://www.conventionalcommits.org/en/about/#tooling-for-conventional-commits)
and uses local git hooks for formatting, linting, branch-name and commit-message checks. See the
[Hooks Guide](.hooks/README.md) for the per-hook detail.

Hook installation is automatic: [cargo-husky](https://github.com/rhysd/cargo-husky) is declared as a dev-dependency of
the `common` crate and copies the hook entrypoints in [`.cargo-husky/hooks/`](.cargo-husky/hooks/) into `.git/hooks/`
the first time you run `cargo test` (or any cargo command that compiles dev-dependencies). No `git config` step or
extra tool install is needed.

The hooks shell out to the helper scripts in [`.hooks/`](.hooks/), which is the single source of truth for fmt + sort
+ clippy invocations across all workspaces — CI calls the same helpers.

One-time tooling that the hooks rely on:

```bash
# Nightly rustfmt for the formatting hook
rustup component add rustfmt --toolchain nightly

# cargo-sort for Cargo.toml normalisation
cargo install cargo-sort
```

If you ever need to reinstall the hooks manually (e.g. you wiped `.git/hooks/`), bump cargo-husky's version pin in
`common/Cargo.toml` or run `cargo clean -p cargo-husky` and then `cargo test --no-run`.

### Migrating from the previous `rusty-hook` setup

If you cloned the repo before this migration, your `.git/hooks/` directory still contains `rusty-hook` stubs for hook
events the new setup does not use (`post-commit`, `post-merge`, etc.). They will try to auto-install `rusty-hook` on
every commit. To clean up:

```bash
# Remove every hook file installed by either runner so the next cargo test
# triggers a clean reinstall via cargo-husky.
find .git/hooks -type f ! -name '*.sample' -delete
cargo test --no-run -p common
```

Once the new hooks are in place you can also `cargo uninstall rusty-hook` if you previously installed it globally.