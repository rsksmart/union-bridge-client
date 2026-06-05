# Dependabot Batch Merge

Process a backlog of open Dependabot PRs in one pass. Trigger: **"follow the dependabot batch merge"**.

> This is a decision spec, not a tutorial — work out the obvious mechanics yourself,
> but follow the explicit rules below; they're the parts that change the outcome.
> Branch protection, required-check names, and the merge mechanism drift over time —
> **discover them at run time**, don't trust values hardcoded in a doc.

> **Human gate — do not skip.** Steps 1–3 run autonomously; **step 4 does not.**
> Never approve, merge, or close any PR until the human explicitly confirms this run.
> Approving on the human's behalf is opt-in per run — never the default. When unsure, stop and ask.

## 1. Classify each PR — the gate differs

| Kind | Touches | Validation |
| --- | --- | --- |
| Rust dep | `Cargo.toml` / `Cargo.lock` | build + test + lint, then the happy-path (step 2) |
| Docker image | a `Dockerfile` | build the images + docker-anvil happy-path (step 2) |
| GitHub Actions | `.github/workflows/*` or `.github/actions/*` | confirm the bump only (e.g. pinned SHA matches its tag); no build |

## 2. Validate locally

Stack candidates onto a throwaway branch off `main` (**don't push it**). Drop any PR
that fails and record why; **don't try to fix ecosystem-wide skews** (e.g. `alloy-*`
crates that must bump together) — reject and move on.

- **Rust:** after each merge run build, test, the lint/style hooks (`bash .hooks/format-code.sh --check` and
  `bash .hooks/check-lints.sh` — fmt/sort/clippy `--locked` across all workspaces, mirroring the required
  `style` check, and catching per-workspace lockfile drift), plus `cargo audit` on the root and `cli`
  lockfiles (the `audit.yml` gate). That isolates breakers per PR; then
  run the Automated Happy-Path **once** on the assembled branch — see
  [`LOCAL_SETUP.md`](LOCAL_SETUP.md#automated-happy-path). If the combined happy-path fails
  (a PR that built + tested fine but breaks the flow only in combination), **bisect** the
  stacked PRs to pinpoint the breaker, drop it, and re-run on the rest.
- **Docker image:** build the builder **and** service images (see
  [`docker/build/README.md`](../docker/build/README.md)) and run the happy-path against
  the docker stack — see [`LOCAL_SETUP.md` › Mode: All Docker](LOCAL_SETUP.md#mode-all-docker).
  (The builder image is `linux/amd64`; risc0/`rzup` can't build on arm64, so that part
  is CI-only.)

Resolve `Cargo.lock` conflicts by **regenerating** (`cargo update -p <crate> --precise <ver>`), never by
hand-merging lines — a line-merged lockfile can look clean but be invalid.

## 3. Pick the merge mechanism — discover, don't assume

Read the repo's branch protection / rulesets, then:

- merge queue enabled → add PRs through the queue (e.g. the `enqueuePullRequest` GraphQL mutation — verify it's still current)
- else auto-merge allowed → `gh pr merge <n> --auto`
- else → `gh pr merge <n>`

If the queue/protection pins one merge method, use it; if several are allowed
(merge / squash / rebase), **ask which to use**.

## 4. Propose, then wait for the human

Reconcile against the live **required** checks (ignore non-required ones). Present three
buckets and ask — *"if you're ok, I'll run these:"* — and **state the merge method** you
resolved in step 3 (e.g. "add to the merge queue (squash)") so it's confirmed too.

- **Ready** — required checks green and validated locally → approve + merge via that method
- **Check** — validated locally but a required check is red → leave it; if it looks
  flaky, say so and ask whether to re-run. If you re-run, ask whether to keep monitoring
  it (in the background, reporting back when it resolves — re-proposing the PR if it goes
  green); monitoring lasts only while the session is alive.
- **Reject** — failed local validation → close, with the reason

Also surface, so the human can validate before confirming:

- the PR diff is **only** the bump — no unexpected extra changes (matters most for SHA-pinned actions)
- any **major / semver-risky** bump — a green build + happy-path proves it compiles and the flow works, not that behaviour is unchanged
- for **Check** items, *why* the red check looks flaky — don't re-run a genuine failure

**Hard stop: don't approve, merge, or close anything before the human confirms** (see the
Human gate at the top). Approving on the human's behalf is a per-run instruction they give
explicitly — never the default.
