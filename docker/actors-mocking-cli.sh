#!/usr/bin/env bash

# Get container ID and attach to it
container_id=$(docker compose ps -q actors-mocking)
docker attach "$container_id"