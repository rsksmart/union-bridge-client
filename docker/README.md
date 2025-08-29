# Setup

Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that
will be automatically used by the Docker Compose setup.

## Shell Scripts

The docker directory contains several shell scripts to help manage Docker operations for the Union client:

### d-compose-cli.sh - Main Build & Run Script

`d-compose-cli.sh` provides a unified CLI tool for building and running Union client services with Docker Compose.

For detailed usage, commands, options, and examples:
```bash
bash d-compose-cli.sh --help
```

### d-multi-client.sh - Multi-Client Automation

`d-multi-client.sh` automatically starts all 4 Union client instances with pre-configured network settings.

#### Usage

```bash
bash d-multi-client.sh
```

This script will start clients 1-4 in detached mode with the following configuration:
- Client 1: IPs 172.25.1.10-15
- Client 2: IPs 172.25.2.10-15  
- Client 3: IPs 172.25.3.10-15
- Client 4: IPs 172.25.4.10-15

**Note:** Ensure the Docker network exists before running:
```bash
docker network create union-bridge --subnet 172.25.0.0/22
```

### d-build-builder.sh - Builder Image Script

`d-build-builder.sh` builds the Union client base builder images.

For detailed usage and examples:
```bash
bash d-build-builder.sh --help
```

### Registry Management Scripts

#### pull_from_ghcr.sh
Pulls all Union client images from GitHub Container Registry.

```bash
bash d-ghcr-pull.sh
```

**Prerequisites:** Login to GHCR first:
```bash
echo $GITHUB_REGISTRY_TOKEN | docker login ghcr.io -u <your_user> --password-stdin
```

#### push_to_ghcr.sh  
Tags and pushes all Union client images to GitHub Container Registry.

```bash
bash d-ghcr-push.sh
```

**Prerequisites:** Same as pull script - requires GHCR authentication.

## Config

By default, the compose will use:

- the `config/docker` folder for the Union Client config files
    - you can override this by passing environment variables prefixed with `UB__` matching the config structure, e.g.
      `UB__block_broker__ip=...`
- the `docker/.env` for `docker-compose` file environment variables

## Multiclient

### Quick Start

For a quick multi-client setup, use the automated script:

```bash
# Ensure network exists
docker network create union-bridge --subnet 172.25.0.0/22

# Start all 4 clients automatically
bash d-multi-client.sh
```

### Manual Setup

If you need more control or want to run clients individually:

**Notes:**

- The current implementation `rust-bitvmx-broker` crate only allows instantiating the `BrokerConfig` with IpAddr type,
  meaning we cannot use container names as hostnames. This has one implication: that we need to define IPs in the
  `docker-compose.yml` file, and these IPs need to be in the same subnet. This is the reason why we have the different
  `docker-compose.*.yml` files within `multiclient` folder, overriding only service IPs over the base
  `docker-compose.yml` file.
- The multi-client swarm will run under the same Docker network.

**Manual Steps:**

1. Go to `docker` folder
2. Run `docker network create union-bridge --subnet 172.25.0.0/22` if the network does not exist yet
3. Run client 1: `BLOCK_BROKER_HOST=172.25.1.10 LOG_BROKER_HOST=172.25.1.11 USER_BROKER_HOST=172.25.1.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-1 -f docker-compose.yml -f multiclient/docker-compose.1.yml up`
4. Run client 2: `BLOCK_BROKER_HOST=172.25.2.10 LOG_BROKER_HOST=172.25.2.11 USER_BROKER_HOST=172.25.2.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-2 -f docker-compose.yml -f multiclient/docker-compose.2.yml up`
5. Run client 3: `BLOCK_BROKER_HOST=172.25.3.10 LOG_BROKER_HOST=172.25.3.11 USER_BROKER_HOST=172.25.3.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-3 -f docker-compose.yml -f multiclient/docker-compose.3.yml up`
6. Run client 4: `BLOCK_BROKER_HOST=172.25.4.10 LOG_BROKER_HOST=172.25.4.11 USER_BROKER_HOST=172.25.4.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-4 -f docker-compose.yml -f multiclient/docker-compose.4.yml up`

### Custom BitVMX Configuration

If you want to connect to a specific BitVMX Broker, you can override default config values with environment variables:
`BITVMX_HOST=<host> BITVMX_PORT=<port> docker compose -f docker-compose.yml -f multiclient/docker-compose-N.yml up`

## Troubleshooting

Build with `./d-compose-cli.sh build --features=anvil` if you are going to connect to a local anvil node, otherwise you
will face problems with different header formats, etc.