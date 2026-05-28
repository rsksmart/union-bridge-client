# Behavioral Guidelines

## Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If your diff is meaningfully larger than the smallest version that works, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.
- Preserve existing comments when refactoring or moving code — don't drop them as part of the move.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

When writing tests, default to covering the unhappy paths — invalid input, error returns from dependencies, edge cases. The bug usually lives in the case you didn't test.

For multi-step tasks, state the plan briefly with the verification you'll use for each step — concrete checks ("cargo test passes", "endpoint returns 200") beat abstract milestones ("it works").

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

For Rust edits in this repo, "verified" includes both clippy and tests passing. Hooks enforce this on push, but earlier is cheaper — run them locally before declaring work done.

# General

## Agent Style
- Use brief and direct responses
- If you don't know the answer, state it — don't fabricate
- I prefer critical thinking over agreement — challenge me, even when I haven't asked

## Team conventions
- No new TODOs — use Jira tickets instead
- Crate strictness varies — `cli/*` and `user-api` run a relaxed bar (`clippy::pedantic` allowed). See [`CONTRIBUTING.md` › Scope and classification](CONTRIBUTING.md#scope-and-classification).

## Build and Verify

Source of truth for fmt/sort/clippy is [`.hooks/`](.hooks/) — CI and pre-push call the same scripts, and they iterate across `.`, `cli/`, and `check-fork/zkp/guest/`. Call them when you need CI parity:

```bash
bash .hooks/format-code.sh --check   # canonical fmt + sort check across all workspaces
bash .hooks/check-lints.sh           # canonical clippy check across all workspaces
```

For faster inner-loop checks against the root workspace only:

```bash
cargo build --workspace                                                                            # full build
RISC0_SKIP_BUILD=1 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings   # lint (matches hooks except multi-workspace iteration)
cargo test --workspace --locked                                                                    # all tests
cargo test --package <crate> <test_name>                                                           # single test
bash .hooks/format-code.sh                                                                         # write mode (rewrites files); add --check to verify only
```

**Don't build, test, or clippy after every edit.** Run them at meaningful milestones — after completing a logical change, before declaring work done, or when debugging a specific failure. Mid-edit verification slows the loop; pre-push hooks enforce clippy + tests at push time anyway.

# Coordinator and Flows

The `coordinator` crate is the orchestration daemon and the entry point for most flow-related questions. It dispatches four event types to every registered `EventProcessor` each tick:

- Rootstock contract events (from `log-indexer`) — contract source in the `union-bridge-contracts` sibling repo: `src/`
- BitVMX broker messages — protocol detail in the `rust-bitvmx-client` sibling repo: `examples/union/`, `src/program/protocols/union/`
- New Rootstock block headers (from `block-indexer`)
- User requests (forwarded from `user-api`)

Each flow is an `EventProcessor` with a persistent state machine. Flows live under [`coordinator/src/flows/`](coordinator/src/flows/):

- `committee/` — committee setup (member keys, communication data, BitVMX dispute channels)
- `pegin/` — peg-in (Bitcoin deposit → Rootstock accepted pegin)
- `pegout/` — pegout user-take (Rootstock request → Bitcoin user-take → Rootstock registration)
- `operator_take/` — fallback when user-take signatures time out
- `btc_signature/` — MuSig2 nonce/signature coordination subflow used by `pegin/` and `pegout/`

Understanding a flow end-to-end usually requires reading across all three repos: this one (orchestration), `rust-bitvmx-client` (BitVMX side), and `union-bridge-contracts` (Rootstock side). The dispatch list above shows where each side enters the coordinator.

See [`docs/e2e/`](docs/e2e/README.md) for sequence diagrams, BitVMX message catalogs, Rootstock event mappings, and timeout rules.

# Project Pointers

Agent guidance lives here. For everything else, consult the canonical docs — organized by when you need them:

- **Before committing or opening a PR** — [`.hooks/README.md`](.hooks/README.md) (commit-message format, branch-name rules) and [`.github/pull_request_template.md`](.github/pull_request_template.md).
- **Writing new code or unsure about a rule** — [`CONTRIBUTING.md`](CONTRIBUTING.md) (engineering standards: error handling, defensive coding, observability, concurrency, unsafe policy, codebase-specific rules).
- **Local dev setup, env vars, troubleshooting** — [`LOCAL_SETUP.md`](LOCAL_SETUP.md).

# Codebase-Specific Patterns

Concrete patterns this repository expects for areas where [`CONTRIBUTING.md`](CONTRIBUTING.md) defines the rule.

## Visibility

> See [`CONTRIBUTING.md` › Workspace and crate boundaries](CONTRIBUTING.md#workspace-and-crate-boundaries) for
> the rule. The bullets below are the concrete checks reviewers run when verifying compliance.

- Prefer the minimum visibility that satisfies actual callers. Use no visibility marker for items used only within their own module; `pub(crate)` only when the item is used across modules of the same crate; `pub` only when the item is reached from another crate.
- The `unreachable_pub` lint is enforced workspace-wide and catches `pub` items that could be `pub(crate)`. The `pub(crate) → private` direction has no automated check; verify introduced or modified items by hand at review time.
- Existing items in the codebase may be over-visible. When you touch a file, tighten the visibility of the items you modify rather than carrying their previous visibility forward.

## Secrets in Types

> See [`CONTRIBUTING.md` › Configuration and secrets](CONTRIBUTING.md#configuration-and-secrets) for the rule.
> Reviewers check the concrete cases below when touching secret-holding types.

- Stored secrets use `SecretString` or `SecretBox<T>`, including external secret types without redacted `Debug`.
- `.expose_secret()` appears only at the consuming boundary.
- Transient secret parameters are not stored in `Debug`-deriving types; prefer `&SecretString` across module boundaries.
