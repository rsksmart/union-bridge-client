# GitHub Actions Workflows

This file is the narrow reference for the workflow files currently tracked under `.github/workflows/`.

## Workflow Set

### `docker-release.yml`

- Name: `Build and Push Docker Images on Release`
- Trigger: push tags matching `v*`
- Purpose: build and push `block-indexer`, `log-indexer`, `coordinator`, and `user-api` images to GHCR
- Variants: `prod` and `anvil`

### `tests.yml`

- Name: `Tests`
- Triggers:
  - pull requests (`opened`, `synchronize`, `reopened`, `ready_for_review`)
  - merge queue checks for `main`
- Jobs:
  - `test-workspace`
  - `check-all-targets`

### `style.yml`

- Name: `Style`
- Triggers:
  - pull requests (`opened`, `synchronize`, `reopened`, `ready_for_review`)
  - merge queue checks for `main`
- Purpose: rustfmt, cargo-sort, and clippy checks

### `codeql.yml`

- Name: `CodeQL-Advanced`
- Triggers:
  - pull requests against `main`
  - merge queue checks for `main`
  - pushes to `main`
- Current matrix: Actions only; Rust remains commented out in the workflow

### `semgrep.yml`

- Name: `Semgrep Security Scan`
- Triggers:
  - pushes to `main`
  - pull requests against `main`
  - scheduled weekly scan

### `check_peer_tested.yml`

- Name: `Check Peer Tested`
- Triggers:
  - pull requests against `main`
  - actions: `opened`, `synchronize`, `reopened`, `labeled`, `unlabeled`
- Purpose: maintain the `Peer Test Status Check` commit status from the `peer tested` label

### `e2e-smoke-tests.yml`

- Name: `E2E Smoke Tests`
- Triggers:
  - pull requests against `main`
  - actions: `opened`, `synchronize`, `reopened`, `ready_for_review`
  - `workflow_dispatch`
- Important runtime rule: draft PRs do not execute the job
- Setup and cross-repo secret ownership: see the [E2E Test Setup Guide](./E2E-TESTS-SETUP.md)

## Local Testing with `act`

`act` is useful for lightweight workflow debugging, but do not assume it perfectly reproduces GitHub-hosted runners.

```bash
# Install act
brew install act

# First-time setup
cp .actrc.sample .actrc

# Pull-request style run
act pull_request --container-architecture linux/amd64

# Dry-run the release workflow
act push --workflows .github/workflows/docker-release.yml --dryrun

# Run the tests workflow file explicitly
act pull_request --workflows .github/workflows/tests.yml --container-architecture linux/amd64

# Sequential runs with container reuse
act pull_request --container-architecture linux/amd64 --concurrent-jobs 1 --reuse
```

Adjust `.actrc` to match your local Docker/runner preferences before relying on it for repeated workflow debugging.

## Secrets and Auth Notes

- `E2E_FRAMEWORK_GITHUB_TOKEN`: token used by `e2e-smoke-tests.yml` to dispatch the external E2E workflow. In practice
  this is usually a PAT with access to `union_bridge_e2e_framework` and its workflows.
- Local `act` runs can still diverge from GitHub-hosted runs, especially when private-repo access or cross-repo
  dispatch is involved.

## Troubleshooting

- `docker-release.yml` did not start: confirm the pushed tag matches the `v*` pattern used by the workflow
- missing `e2e-smoke-tests` status on a PR: confirm the PR targets `main` and is not draft
- `docker-release.yml` images are built for `linux/amd64` only; this workflow does not currently publish ARM variants
