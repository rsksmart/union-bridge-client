# E2E Smoke Tests - Quick Reference

> **📖 For complete documentation**, see: [`union_bridge_e2e_framework/.github/workflows/E2E-SMOKE-TESTS.md`](https://github.com/rsksmart/union_bridge_e2e_framework/blob/main/.github/workflows/E2E-SMOKE-TESTS.md)

## Quick Start

This workflow (`e2e-smoke-tests.yml`) automatically triggers E2E smoke tests when a PR is opened or updated. Tests run **before** the PR can be merged.

### Manual Triggering

```bash
# From client repo - simplest (defaults to current commit)
gh workflow run e2e-smoke-tests.yml

# Or specify a branch/tag/commit SHA
gh workflow run e2e-smoke-tests.yml -f client_ref=feature/my-branch
```

### Setup Checklist

1. **Configure branch protection** (in `union-bridge-client` repo):
   - Settings → Branches → Add rule for `main`
   - Require status check: `e2e-smoke-tests`

2. **Configure secrets** (in `union_bridge_e2e_framework` repo):
   - `TOKEN_CONTRACTS`
   - `TOKEN_FAIRGATE`
   - `USER_BITCOIN_WIF`
   - `MEMBER_BITCOIN_WIF`

## How It Works

1. PR opened/updated → Client workflow triggers
2. Status check created → Set to `pending`
3. E2E tests run → In `union_bridge_e2e_framework` repo
4. Status check updated → `success` or `failure`
5. Merge can be blocked → If status check fails (when branch protection is enabled)

**Note**: Currently, the E2E smoke test check is optional and can be bypassed. Branch protection enforcement will be enabled in the future to require this check before merging.
