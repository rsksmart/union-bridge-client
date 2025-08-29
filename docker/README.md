# Setup

Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that
will be automatically used by the Docker Compose setup.

## Compose CLI

`compose-cli.sh` provides an unified CLI tool for managing Docker Compose operations for the Union client.

### Usage

The CLI help menu should provide all the information you need on how to build and run the services, including features
and mocking management, and some examples.

```bash
bash compose-cli.sh --help
```

## Config

By default, the compose will use:

- the `config/docker` folder for the Union Client config files
    - you can override this by passing environment variables prefixed with `UB__` matching the config structure, e.g.
      `UB__block_broker__ip=...`
- the `docker/.env` for `docker-compose` file environment variables

## Multiclient

Notes:

- The current implementation `rust-bitvmx-broker` crate only allows instantiating the `BrokerConfig` with IpAddr type,
  meaning we cannot use container names as hostnames. This has one implication: that we need to define IPs in the
  `docker-compose.yml` file, and these IPs need to be in the same subnet. This is the reason why we have the different
  `docker-compose.*.yml` files within `multiclient` folder, overriding only service IPs over the base
  `docker-compose.yml` file.
- The multi-client swarm will run under the same Docker network.

Now, to run the multi-client you have to:

1. go to `docker` folder
2. run `docker network create union-bridge --subnet 172.25.0.0/22` if the network does not exist yet
3. run client 1: `BLOCK_BROKER_HOST=172.25.1.10 LOG_BROKER_HOST=172.25.1.11 USER_BROKER_HOST=172.25.1.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-1 -f docker-compose.yml -f multiclient/docker-compose.1.yml up`
4. run client 2: `BLOCK_BROKER_HOST=172.25.2.10 LOG_BROKER_HOST=172.25.2.11 USER_BROKER_HOST=172.25.2.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-2 -f docker-compose.yml -f multiclient/docker-compose.2.yml up`
5. run client 3: `BLOCK_BROKER_HOST=172.25.3.10 LOG_BROKER_HOST=172.25.3.11 USER_BROKER_HOST=172.25.3.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-3 -f docker-compose.yml -f multiclient/docker-compose.3.yml up`
6. run client 4: `BLOCK_BROKER_HOST=172.25.4.10 LOG_BROKER_HOST=172.25.4.11 USER_BROKER_HOST=172.25.4.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-4 -f docker-compose.yml -f multiclient/docker-compose.4.yml up`


If you want to connect to a specific BitVMX Broker, you can override default config values with environment variables:
`BITVMX_HOST=<host> BITVMX_PORT=<port> docker compose -f docker-compose.yml -f multiclient/docker-compose-N.yml up`

## Troubleshooting

Build with `./compose-cli.sh build --feature-anvil` if you are going to connect to a local anvil node, otherwise you
will face problems with different header formats, etc.