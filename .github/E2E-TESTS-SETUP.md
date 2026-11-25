# E2E Smoke Tests

This document explains how this repo's `e2e-smoke-tests.yml` workflow triggers smoke tests in
`union_bridge_e2e_framework`.

Flow: client workflow runs, dispatches the external E2E workflow, the E2E repo executes the smoke test, and then
updates the `e2e-smoke-tests` status on the client PR.

## Trigger Rules

- pull requests to `main`
- PR actions: `opened`, `synchronize`, `reopened`, `ready_for_review`
- manual runs through `workflow_dispatch`

Important: draft PRs do not run the job. The workflow file gates execution with:

- `workflow_dispatch`, or
- `pull_request.draft == false` and the PR branch does not start with `dependabot/github_actions/`

Dependabot cargo and docker PRs run the smoke like any other PR and get an `e2e-smoke-tests` status. Their runs receive
Dependabot secrets instead of Actions secrets, so `E2E_FRAMEWORK_GITHUB_TOKEN` must be stored under both (see Setup).
If the Dependabot copy is missing, the job sets the status to `error` and says so instead of attempting the dispatch.

Dependabot **github-actions** PRs (branch `dependabot/github_actions/…`) are skipped on purpose and get **no**
`e2e-smoke-tests` status. Such a PR may bump `actions/github-script`, the action this job runs with
`E2E_FRAMEWORK_GITHUB_TOKEN`, and its `pull_request` run would execute that unreviewed action code with the token.
Cargo and docker bumps run the smoke.

## Refs

| Ref | PR run | Manual run |
| --- | --- | --- |
| client | PR head SHA; not overridable | `client_ref`, empty means current commit |
| e2e | PR label `e2e-ref:<ref>` or default `main` | `e2e_ref`, empty means `main` |
| status target SHA | PR head SHA | `pr_head_sha`, empty means `github.sha` |

## Trusted Actors

Only trusted maintainers can override the E2E framework ref with:

- PR label `e2e-ref:<ref>`
- manual input `e2e_ref`

If a non-trusted actor tries to set either one, the workflow fails and sets the `e2e-smoke-tests` commit status to
`error`.

On a Dependabot PR, a PR label `e2e-ref:<ref>` ends in `error` whenever Dependabot triggers the run (its own pushes and
rebases): the actor is `dependabot[bot]`, which is not in the trusted team. To test a Dependabot PR against another E2E
ref, a trusted maintainer runs the workflow manually **from `main`** (never from the Dependabot branch, whose bumped
actions would run with the dispatch secrets), with `e2e_ref` set to the E2E framework branch or tag, and `client_ref`
and `pr_head_sha` both set to the Dependabot PR's head SHA.

## Setup

1. In `union-bridge-client`, configure `E2E_FRAMEWORK_GITHUB_TOKEN`. For cross-repo dispatch this is typically a GitHub
   Personal Access Token (PAT) with `repo` and `workflow` access to `union_bridge_e2e_framework`. Store it as a
   **Dependabot** secret too (Settings → Secrets and variables → Dependabot), so Dependabot PRs can dispatch the smoke.
2. Optionally require the `e2e-smoke-tests` status check in branch protection for `main`. If you do, Dependabot
   github-actions PRs, which never get that status, can only merge with an admin bypass.
3. In `union_bridge_e2e_framework`, configure the secrets required by that pipeline:
   - `USER_BITCOIN_WIF`
   - `MEMBER_BITCOIN_WIF`
   - optional `GHCR_USERNAME` if the PAT owner differs from `github.actor`

Full external E2E smoke-tests guide:
[External E2E Smoke Tests Guide](https://github.com/rsksmart/union_bridge_e2e_framework/blob/main/.github/workflows/E2E-SMOKE-TESTS.md)
