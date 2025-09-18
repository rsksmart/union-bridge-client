# GitHub Actions Workflows

This directory contains GitHub Actions workflows for the Union Bridge Client project.

## Workflows

### Docker Release (`docker-release.yml`)

**Purpose**: Automatically builds and pushes Docker images to GitHub Container Registry (GHCR) when a new Git tag is created.

**Triggers**:
- Push to tags matching `v*` pattern (e.g., `v1.0.0`, `v2.1.3`)
- Manual workflow dispatch with custom tag input

**What it does**:
1. Builds 4 Docker images: `block-indexer`, `log-indexer`, `coordinator`, `user-api`
2. Tags images with the Git tag name (e.g., `v1.0.0`)
3. Pushes images to `ghcr.io/rsksmart/union-client-*`
4. Updates `latest` tags for the new version

**Required Secrets**:
- `FAIRGATE_GITHUB_TOKEN`: GitHub token for accessing FairgateLabs repositories
- `UNION_CONTRACTS_GITHUB_TOKEN`: GitHub token for accessing temp-rsk contracts repository
- `GITHUB_TOKEN`: Automatically provided by GitHub for GHCR authentication

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
- `ghcr.io/rsksmart/union-client-block-indexer:v1.0.0`
- `ghcr.io/rsksmart/union-client-log-indexer:v1.0.0`
- `ghcr.io/rsksmart/union-client-coordinator:v1.0.0`
- `ghcr.io/rsksmart/union-client-user-api:v1.0.0`

Plus `latest` tags for each image.

### Other Workflows

- `crates_tests.yml`: Runs Rust tests on pull requests
- `codeql.yml`: Security analysis with CodeQL
- `check_peer_tested.yml`: Peer testing workflow
- `semgrep.yml`: Additional security scanning

## Setup Requirements

### GitHub Token Setup

The Docker build process requires access to private repositories using GitHub tokens (same as other workflows in this project):

1. **FAIRGATE_GITHUB_TOKEN**: Already configured for FairgateLabs repositories
2. **UNION_CONTRACTS_GITHUB_TOKEN**: Already configured for temp-rsk contracts repository

These tokens are already set up and used by other workflows in the project.

### GHCR Authentication

The workflow uses `GITHUB_TOKEN` for GHCR authentication, which is automatically provided by GitHub Actions. No additional setup required.

## Troubleshooting

### Build Failures

- Check that SSH key has access to required private repositories
- Verify that all dependencies are properly specified in `Cargo.toml`
- Check GitHub Actions logs for specific error messages

### Permission Issues

- Ensure the repository has `packages: write` permission
- Verify that the GitHub token has sufficient permissions

### Tag Issues

- Use semantic versioning format: `v1.0.0`, `v2.1.3`, etc.
- Avoid using `latest` as a tag name (reserved for automatic latest updates)