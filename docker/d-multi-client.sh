#!/usr/bin/env bash

BLOCK_BROKER_HOST=172.25.1.10 LOG_BROKER_HOST=172.25.1.11 USER_BROKER_HOST=172.25.1.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-1 -f docker-compose.yml -f multiclient/docker-compose.1.yml up -d
BLOCK_BROKER_HOST=172.25.2.10 LOG_BROKER_HOST=172.25.2.11 USER_BROKER_HOST=172.25.2.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-2 -f docker-compose.yml -f multiclient/docker-compose.2.yml up -d
BLOCK_BROKER_HOST=172.25.3.10 LOG_BROKER_HOST=172.25.3.11 USER_BROKER_HOST=172.25.3.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-3 -f docker-compose.yml -f multiclient/docker-compose.3.yml up -d
BLOCK_BROKER_HOST=172.25.4.10 LOG_BROKER_HOST=172.25.4.11 USER_BROKER_HOST=172.25.4.15 BITVMX_HOST=192.168.65.254 BITVMX_PORT=61180 docker compose -p uc-4 -f docker-compose.yml -f multiclient/docker-compose.4.yml up -d
