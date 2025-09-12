# Docker Setup for Union Client

## Config

Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that
will be automatically used by the Docker Compose setup.

By default, the compose will use:

- the `config/docker` folder for the Union Client config files
    - you can override any configuration value using environment variables prefixed with `UB__` matching the config structure, e.g.
      `UB__block_broker__ip=192.168.1.100`
    - see the main [README.md](../README.md#configuration-overrides) for detailed examples and mapping rules
- the `docker/.env` for `docker-compose` file environment variables

## Build Builder Images

### d-build-builder.sh - Builder Image Script

`d-build-builder.sh` builds the Union client base builder images.

For detailed usage and examples:
```bash
bash d-build-builder.sh --help
```

## Build & Run Services

The docker directory contains several shell scripts to help manage Docker operations for the Union client:

### d-compose-cli.sh - Main Build & Run Script

`d-compose-cli.sh` provides a unified CLI tool for building and running Union client services with Docker Compose.

For detailed usage, commands, options, and examples:
```bash
bash d-compose-cli.sh --help
```

### First time docker setup pre-requisite 
Inside the docker directory, copy the contents of the `.env.sample` file into a new `.env` file (not to be confused with `.envrc` which the project uses in the root directory). 

Set a value for the `KEY_STORE_PASSWORD` variable, it doesn't need to be the same as the equivalent var in the `.envrc` of the project's root directory.

N.B.: Please note that you need to have both Anvil and the `bitvmx-workspace` running before starting up the Union client services with Docker.

### d-multi-client.sh - Multi-Client Automation

`d-multi-client.sh` automatically starts all 4 Union client instances with different BitVMX broker ports.

#### Usage

```bash
bash d-compose-cli.sh build --features=anvil
bash d-multi-client.sh
```

This script will start clients 1-4 in detached mode with the following BitVMX broker configuration:
- Client 1: BitVMX port 22222
- Client 2: BitVMX port 33333  
- Client 3: BitVMX port 44444
- Client 4: BitVMX port 55554

Each client runs as a separate Docker Compose project (uc-1, uc-2, uc-3, uc-4) using the main `docker-compose.yml` file.

## Multiclient

### Quick Start

For a quick multi-client setup, use the automated script:

```bash
# Start all 4 clients automatically
bash d-multi-client.sh
```

This will start 4 separate Union client instances, each connecting to a different BitVMX broker port (22222, 33333, 44444, 55554).

### Manual Setup

If you need more control or want to run clients individually:

**Manual Steps:**

1. Go to `docker` folder
2. Run individual clients with different BitVMX broker configurations:
   ```bash
   # Client 1
   BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 up -d
   
   # Client 2  
   BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 up -d
   
   # Client 3
   BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 up -d
   
   # Client 4
   BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 up -d
   ```

### Custom BitVMX Configuration

You can connect to any BitVMX Broker by specifying the host and port:
```bash
BITVMX_HOST=<host> BITVMX_PORT=<port> docker compose -p <project-name> up
```

### Configuration Templates

For more advanced multi-client setups, you can use the configuration templates located in `../config/multi-client-template/`. These templates provide pre-configured settings for 4 different clients with unique broker ports and client IDs.

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

Build with `./d-compose-cli.sh build --features=anvil` if you are going to connect to a local anvil node, otherwise you
will face problems with different header formats, etc.