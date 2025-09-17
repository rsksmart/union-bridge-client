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
- `SSH_PRIVATE_KEY`: SSH private key for accessing private repositories
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

### SSH Key Setup

The Docker build process requires access to private repositories. Set up an SSH key:

1. Generate SSH key pair:
   ```bash
   ssh-keygen -t ed25519 -C "github-actions@rsksmart.com" -f ~/.ssh/github_actions
   ```

2. Add public key to GitHub:
   - Go to repository Settings → Deploy keys
   - Add the public key (`github_actions.pub`)

3. Add private key to GitHub Secrets:
   - Go to repository Settings → Secrets and variables → Actions
   - Add secret `SSH_PRIVATE_KEY` with the private key content

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
