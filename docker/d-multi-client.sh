#!/usr/bin/env bash

ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 -f docker-compose.yml up -d
ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 -f docker-compose.yml up -d
ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 -f docker-compose.yml up -d
ROOTSTOCK_URL=ws://host.docker.internal:8545 BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 -f docker-compose.yml up -d
