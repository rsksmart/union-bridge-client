#!/usr/bin/env bash

BITVMX_HOST=host.docker.internal BITVMX_PORT=22222 docker compose -p uc-1 -f docker-compose.yml up -d
BITVMX_HOST=host.docker.internal BITVMX_PORT=33333 docker compose -p uc-2 -f docker-compose.yml up -d
BITVMX_HOST=host.docker.internal BITVMX_PORT=44444 docker compose -p uc-3 -f docker-compose.yml up -d
BITVMX_HOST=host.docker.internal BITVMX_PORT=55554 docker compose -p uc-4 -f docker-compose.yml up -d
