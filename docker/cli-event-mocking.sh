#!/usr/bin/env bash

# Get container ID and attach to it
container_id=$(docker compose ps -q sc-event-mocking)
docker attach "$container_id"