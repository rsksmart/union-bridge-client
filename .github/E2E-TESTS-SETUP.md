# E2E Smoke Tests

This workflow triggers E2E smoke tests in `union_bridge_e2e_framework`. 
- On PR open/update it runs automatically and posts a status check (`e2e-smoke-tests`). 
- Manual runs: Actions → E2E Smoke Tests → Run workflow.

**Flow:** Client workflow runs → calls e2e repo via API → e2e workflow runs tests → updates status check on the PR.

## Refs (what gets tested)

| Ref | PR run | Manual run |
| ----- | ----- | ----- |
| **client** | PR head (branch/SHA). Not overridable. | Input `client_ref`. Empty = current commit. |
| **e2e** (which e2e version runs) | PR label `e2e-ref:<ref>` (e.g. `e2e-ref:v0.2.0`). No label = `main`. | Input `e2e_ref`. Empty = `main`. |
| **contracts** | Derived from client Cargo.toml (union-contracts tag). No override. | — |

**When to add `e2e-ref:<ref>`:** If your client branch needs a different e2e version (e.g. it breaks current e2e tests and you have an e2e branch that supports it), add the PR label `e2e-ref:<ref>`. Otherwise the run uses e2e’s `main`. This lets multiple client branches use different e2e refs without changing code.

**e2e-ref override and trusted actors:** Only trusted maintainers can use the `e2e-ref` label or the `e2e_ref` manual input. If someone else adds the label or sets the input, the workflow fails with an error and does not run the e2e pipeline. This avoids untrusted contributors from choosing which e2e framework ref (and thus which workflow code) runs in the e2e repo. The allowlist is maintained in the client workflow file (`e2e-smoke-tests.yml`).

## Setup

1. **union-bridge-client:** Secret `E2E_FRAMEWORK_GITHUB_TOKEN` (PAT with `repo` + `workflow`, access to `union_bridge_e2e_framework`). Needed to trigger the e2e workflow (else 404). Already set by QA team.
2. **union-bridge-client:** Branch protection for `main` → require status check `e2e-smoke-tests` (optional until enforced).
3. **union_bridge_e2e_framework:** Secrets `TOKEN_CONTRACTS`, `TOKEN_FAIRGATE`, `USER_BITCOIN_WIF`, `MEMBER_BITCOIN_WIF`. Optional: `GHCR_USERNAME` if PAT owner differs from `github.actor`.

Full guide (client + e2e secrets, tokens, multi-arch contracts image): [union_bridge_e2e_framework/.github/workflows/E2E-SMOKE-TESTS.md](https://github.com/rsksmart/union_bridge_e2e_framework/blob/main/.github/workflows/E2E-SMOKE-TESTS.md).
