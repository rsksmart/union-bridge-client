# GitHub Actions Workflows

This directory contains GitHub Actions workflows for the Union Bridge Client project.

## Workflows

### Docker Release (`docker-release.yml`)

**Purpose**: Automatically builds and pushes multi-platform Docker images to GitHub Container Registry (GHCR) when a new Git tag is created.

**Triggers**:
- Push to tags matching `v*` pattern (e.g., `v1.0.0`, `v2.1.3`)
- Manual workflow dispatch with custom tag input

**Features**:
- **AMD64 builds**: Currently builds for `linux/amd64` architecture (ARM64 support pending base image update)
- **Comprehensive metadata**: Rich OCI labels with service descriptions and vendor information
- **Automatic tagging**: Version tags and latest tags handled automatically
- **Private repository access**: Uses GitHub tokens for secure access to private dependencies
- **Security optimized**: Tokens are cleared after use to prevent exposure

**What it does**:
1. **Builds 4 Docker images**: `block-indexer`, `log-indexer`, `coordinator`, `user-api`
2. **AMD64 architecture**: Creates images for Intel/AMD x86_64 architecture
3. **Smart tagging**: 
   - Version tags: `v1.0.0` (from Git tag)
4. **Pushes to GHCR**: `ghcr.io/rsksmart/union-client-*`
5. **Rich metadata**: Service-specific labels and descriptions

**Required Secrets**:
- `FAIRGATE_GITHUB_TOKEN`: GitHub token for accessing FairgateLabs repositories
- `UNION_CONTRACTS_GITHUB_TOKEN`: GitHub token for accessing temp-rsk contracts repository
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

**Dockerfile**: Uses `docker/build/Dockerfile.github-actions` - a GitHub Actions optimized version that uses HTTPS authentication instead of SSH for private repository access.

### Local Testing

The workflow can be tested locally using `act`:

```bash
# Install act (if not already installed)
brew install act

# Test the workflow locally
./test-docker-workflow.sh

# Or run specific tests
act push --workflows .github/workflows/docker-release.yml --dryrun
```

**Note**: Local testing requires Docker to be running and may not fully replicate the GitHub Actions environment, especially for private repository access.

### Other Workflows

- `tests.yml`: Runs Rust tests on pull requests
- `style.yml`: Runs code formatting checks and Clippy linting checks on pull requests
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

The workflow uses `REGISTRY_TOKEN` for GHCR authentication. This must be a Personal Access Token with `write:packages` scope. The token should be configured as a repository secret.

## Troubleshooting

### Build Failures

- **Authentication issues**: Verify `FAIRGATE_GITHUB_TOKEN` and `UNION_CONTRACTS_GITHUB_TOKEN` are properly configured
- **Dependency issues**: Check that all dependencies are properly specified in `Cargo.toml`
- **Docker build issues**: Check GitHub Actions logs for specific error messages
- **Private repository access**: Ensure tokens have access to `FairgateLabs` and `temp-rsk` repositories

### Permission Issues

- **GHCR push failures**: Ensure the repository has `packages: write` and `id-token: write` permissions
- **Token permissions**: Verify that `REGISTRY_TOKEN` has `write:packages` scope and other GitHub tokens have sufficient permissions for private repository access
- **Organization settings**: Check that the `rsksmart` organization allows package publishing

### Tag Issues

- **Version format**: Use semantic versioning format: `v1.0.0`, `v2.1.3`, etc.
- **Reserved names**: Avoid using `latest` as a tag name (reserved for automatic latest updates)
- **Tag pattern**: Ensure tags match the `v*` pattern to trigger the workflow

### Multi-Platform Build Issues

- **Architecture support**: The workflow currently builds for `linux/amd64` only (ARM64 support pending base image update)
- **Build time**: Multi-platform builds take longer (typically 20-30 minutes)
- **Resource limits**: Ensure GitHub Actions has sufficient resources for multi-platform builds

### Security Considerations

- **Token exposure**: Tokens are automatically cleared after use in the Dockerfile
- **Secret management**: All sensitive data is passed via GitHub Secrets
- **Docker security**: Images are built with minimal attack surface using Debian slim base