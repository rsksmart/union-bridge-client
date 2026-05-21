# Docker Build

This directory only covers image build and registry operations. It does not own local runtime setup.

For the full local development procedure, start with the [Local Setup Guide](../../LOCAL_SETUP.md). For local Docker
runtime usage after building images, use the [Operator Docker Runtime Guide](../operator/README.md).

## Related Docs

- [Docker Guide](../README.md): Docker doc index
- [Operator Docker Runtime Guide](../operator/README.md): run operators with the built images
- [Local Setup Guide](../../LOCAL_SETUP.md): shared configuration rules and runtime map

## Config

No local `.env` file is required for the supported build flow in this directory.

The build compose uses the repository `config/` tree. Runtime overrides still follow the shared `UB__...`
configuration model documented in the [Local Setup Guide](../../LOCAL_SETUP.md).

For GHCR authentication, this directory uses the local shell variable `GITHUB_REGISTRY_TOKEN`. That local variable is
separate from the OOTB GitHub Actions GITHUB_TOKEN used in workflows.

## Build Builder Images

```bash
# Build builder images
bash d-build-builder.sh --help

# Build service images
bash d-build-client.sh --help

# Optional: enable the anvil feature for local Anvil-connected images
bash d-build-client.sh --features=anvil

# Pull images from GHCR
bash d-ghcr-pull.sh

# Login to GHCR before pushing with the local shell variable
echo "$GITHUB_REGISTRY_TOKEN" | docker login ghcr.io -u <your_user> --password-stdin

# Push images to GHCR
bash d-ghcr-push.sh
```

If you bypass `start-operators.sh` and run `docker compose` manually from this directory, export the required
variables in your shell first (`KEY_STORE_PASSWORD`) and point `KEYSTORE_DIR` at an existing host keystore directory
containing `user` and `member` files.

## Next Step

After building or pulling images, switch to the [Operator Docker Runtime Guide](../operator/README.md) for local runtime commands.
