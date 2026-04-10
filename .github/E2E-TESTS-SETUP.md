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
- `pull_request.draft == false`

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

## Setup

1. In `union-bridge-client`, configure `E2E_FRAMEWORK_GITHUB_TOKEN`. For cross-repo dispatch this is typically a GitHub
   Personal Access Token (PAT) with `repo` and `workflow` access to `union_bridge_e2e_framework`.
2. Optionally require the `e2e-smoke-tests` status check in branch protection for `main`.
3. In `union_bridge_e2e_framework`, configure the secrets required by that pipeline:
   - `TOKEN_CONTRACTS`: PAT for the private contracts dependency used by that workspace
   - `USER_BITCOIN_WIF`
   - `MEMBER_BITCOIN_WIF`
   - optional `GHCR_USERNAME` if the PAT owner differs from `github.actor`

Full external E2E smoke-tests guide:
[External E2E Smoke Tests Guide](https://github.com/rsksmart/union_bridge_e2e_framework/blob/main/.github/workflows/E2E-SMOKE-TESTS.md)
