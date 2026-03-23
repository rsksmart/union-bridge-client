# Docker Setup for Union Client

## Config

Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that
will be automatically used by the Docker Compose setup.

By default, the compose will use:

- the `config/docker` folder for the Union Client config files
    - you can override any configuration value using environment variables prefixed with `UB__` matching the config structure, e.g.
      `UB__COORDINATOR__BLOCKS__HOST=192.168.1.100`
    - see the main [README.md](../../README.md#configuration-overrides) for detailed examples and mapping rules
- the `docker/build/.env` for `docker-compose` file environment variables

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
Inside the docker/build directory, copy the contents of the `.env.sample` file into a new `.env` file (not to be confused with `.envrc` which the project uses in the root directory). 

Set a value for the `KEY_STORE_PASSWORD` variable, it doesn't need to be the same as the equivalent var in the `.envrc` of the project's root directory.

N.B.: Please note that you need to have both Anvil and the `bitvmx-workspace` running before starting up the Union client services with Docker. Runtime startup is handled from [`docker/operator`](../operator/README.md).

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
