# GitHub Actions Workflows

This directory contains GitHub Actions workflows for the Union Bridge Client project.

## Workflows

### Docker Release (`docker-release.yml`)

**Purpose**: Automatically builds and pushes Docker images to GitHub Container Registry (GHCR) when a new Git tag is
created.

**Required Secrets**:

- `UNION_CONTRACTS_GITHUB_TOKEN`: GitHub PAT with read access to the private `union-contracts` repository referenced from `Cargo.toml`
- `REGISTRY_TOKEN`: Personal Access Token with `write:packages` scope for GHCR authentication

**Usage**:

1. **Automatic (recommended)**:
   ```bash
   git tag v1.0.0
   git push origin v1.0.0
   ```

2. **Manual trigger**:
    - Go to Actions tab in GitHub
    - Select "Build and Push Docker Images on Release"
    - Click "Run workflow"
    - Enter custom tag name

**Images created**:

- `ghcr.io/rsksmart/union-client-block-indexer:v1.0.0` (AMD64)
- `ghcr.io/rsksmart/union-client-log-indexer:v1.0.0` (AMD64)
- `ghcr.io/rsksmart/union-client-coordinator:v1.0.0` (AMD64)
- `ghcr.io/rsksmart/union-client-user-api:v1.0.0` (AMD64)

**Dockerfile**: Uses `docker/build/Dockerfile.github-actions` - a GitHub Actions optimized version that uses HTTPS
authentication instead of SSH, with a token only for the private contracts repository.

---

### Tests (`tests.yml`)

**Purpose**: Runs the Rust test suite to ensure code correctness.

---

### Style (`style.yml`)

**Purpose**: Enforces code formatting (`rustfmt`, `cargo-sort`) and linting standards (`clippy`).

**Known limitation**: We use Clippy nightly for some features, and it's not possible to fix a version. Therefore, our
local and CI versions may differ. If that happens and the CI fails, just update the nightly version in your local
environment and push the changes.

---

### CodeQL (`codeql.yml`)

**Purpose**: Security analysis using GitHub's CodeQL engine. Uses custom configuration from
`.github/codeql/codeql-config.yml`.

**Note**: Rust support is currently disabled as it's not production-ready. Only GitHub Actions workflows are scanned.

---

### Semgrep Security Scan (`semgrep.yml`)

**Purpose**: Security scanning using Semgrep with standard Rust rules and custom rules from
`rootstock/semgrep-rules-rust`.

---

### Check Peer Tested (`check_peer_tested.yml`)

**Purpose**: Enforces peer testing requirement before PR merge. A team member must manually add the `peer tested` label
after reviewing/testing the changes. New commits reset this requirement.

## Local Testing with `act`

You can test GitHub Actions locally using the [`act`](https://github.com/nektos/act) tool. This is useful for debugging
workflows before pushing changes.

### Prerequisites

- Docker installed and running
- `act` installed:
  ```bash
  brew install act
  ```

### Initial Setup

**Only the first time you run `act`, or whenever the base image changes**, copy the `.actrc.sample` to `.actrc` and
configure it as needed. This file is used to configure the `act` tool.

### Running Workflows

To run the same actions as the CI runs on pull requests:

```bash
act pull_request -s KEY_STORE_FILE=$(cat <path_to_your_keystore_file>) --container-architecture linux/amd64
```

To run just Crate Tests:

```bash
act -j test-and-lint -s KEY_STORE_FILE=$(cat <path_to_your_keystore_file>) --container-architecture linux/amd64
```

To test the Docker release workflow:

```bash
act push --workflows .github/workflows/docker-release.yml --dryrun
```

### Tips

- **Reuse containers**: Add `--reuse` to reuse previous Docker containers to speed up execution by skipping setup and
  preserving cache, filesystem, and environment state.
- **Concurrency issues**: If you find concurrency errors, try running with `--concurrent-jobs 1` to run the actions
  sequentially.
- **Artifact performance**: Uploading and downloading artifacts is slow locally, but fast on the CI.

**Note**: Local testing requires Docker to be running and may not fully replicate the GitHub Actions environment,
especially for the private contracts repository access.

## Setup Requirements

### GitHub Token Setup

The Docker build process requires access to the private contracts repository using a GitHub token (same as other
workflows in this project):

1. **UNION_CONTRACTS_GITHUB_TOKEN**: Used for the private contracts repository referenced by `union-contracts`

Configure this secret as a repository or organization secret before running the workflows.

### GHCR Authentication

The workflow uses `REGISTRY_TOKEN` for GHCR authentication. This must be a Personal Access Token with `write:packages`
scope. The token should be configured as a repository secret.

## Troubleshooting

### Build Failures

- **Authentication issues**: Verify `UNION_CONTRACTS_GITHUB_TOKEN` is properly configured
- **Dependency issues**: Check that all dependencies are properly specified in `Cargo.toml`
- **Docker build issues**: Check GitHub Actions logs for specific error messages
- **Private repository access**: Ensure the token has access to the `temp-rsk` repository

### Permission Issues

- **GHCR push failures**: Ensure the repository has `packages: write` permissions
- **Token permissions**: Verify that `REGISTRY_TOKEN` has `write:packages` scope and that
  `UNION_CONTRACTS_GITHUB_TOKEN` has read access to the private contracts repository
- **Organization settings**: Check that the `rsksmart` organization allows package publishing

### Tag Issues

- **Version format**: Use semantic versioning format: `v1.0.0`, `v2.1.3`, etc.
- **Reserved names**: Avoid using `latest` as a tag name (reserved for automatic latest updates)
- **Tag pattern**: Ensure tags match the `v*` pattern to trigger the workflow

### Multi-Platform Build Issues

- **Architecture support**: The workflow currently builds for `linux/amd64` only (ARM64 support pending base image
  update)
- **Build time**: Multi-platform builds take longer (typically 20-30 minutes)
- **Resource limits**: Ensure GitHub Actions has sufficient resources for multi-platform builds

### Security Considerations

- **Token exposure**: Tokens are automatically cleared after use in the Dockerfile
- **Secret management**: All sensitive data is passed via GitHub Secrets
- **Docker security**: Images are built with minimal attack surface using Debian slim base
