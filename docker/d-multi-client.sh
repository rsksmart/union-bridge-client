#!/usr/bin/env bash

set -e

# Parse command line arguments
PROFILE=""
ROOTSTOCK_URL="ws://host.docker.internal:8545"

if [[ "$1" == "--profile" && -n "$2" ]]; then
    PROFILE="--profile $2"
    if [[ "$2" == "anvil" ]]; then
        ROOTSTOCK_URL="ws://anvil:8545"
    fi
    shift 2
fi

# Start all 4 clients with the specified profile
ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 -f docker-compose.yml $PROFILE up -d
ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 -f docker-compose.yml $PROFILE up -d
ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 -f docker-compose.yml $PROFILE up -d
ROOTSTOCK_URL=$ROOTSTOCK_URL BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 -f docker-compose.yml $PROFILE up -d
