# Docker Setup for Union Client

This directory is for image building and registry operations only.
Use [`docker/operator`](../operator/README.md) to run Union Client operators in Docker.

## Config

No local `.env` file is required for the supported flow here.
Operator startup is handled from [`docker/operator`](../operator/README.md), not from this directory.

By default, the compose will use:

- the repository `config/` directory for the Union Client config files
    - you can override any configuration value using environment variables prefixed with `UB__` matching the config structure, e.g.
      `UB__COORDINATOR__BLOCKS__HOST=192.168.1.100`
    - see [DEVELOPER.md](../../DEVELOPER.md#configuration) for `UB__` override rules and examples

## Build Builder Images

### d-build-builder.sh - Builder Image Script

`d-build-builder.sh` builds the Union client base builder images.

For detailed usage and examples:
```bash
bash d-build-builder.sh --help
```

## Build Services

The docker/build directory contains several shell scripts to help manage Docker operations for the Union client:

### d-build-client.sh - Main Build Script

`d-build-client.sh` builds Union client service images with Docker Compose.

For detailed usage, commands, options, and examples:
```bash
bash d-build-client.sh --help
```

### First time docker setup pre-requisite 
For normal Docker runtime usage:

1. build images from here with `d-build-client.sh`
2. run operators from [`docker/operator`](../operator/README.md)

If you still run `docker compose` manually from this directory, export any required variables in your shell first, such as `KEY_STORE_PASSWORD`.

## Registry Management Scripts

### d-ghcr-pull.sh
Pulls all Union client images from GitHub Container Registry.

```bash
bash d-ghcr-pull.sh
```

**Prerequisites:** Login to GHCR first:
```bash
echo $GITHUB_REGISTRY_TOKEN | docker login ghcr.io -u <your_user> --password-stdin
```

### d-ghcr-push.sh  
Tags and pushes all Union client images to GitHub Container Registry.

```bash
bash d-ghcr-push.sh
```

**Prerequisites:** Same as pull script - requires GHCR authentication.

## Troubleshooting

Build with `./d-build-client.sh --features=anvil` if you are going to connect to a local anvil node, otherwise you
will face problems with different header formats, etc.
