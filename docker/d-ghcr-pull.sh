#!/usr/bin/env bash

set -e

# TODO Improve receiving parameters and using "docker buildx bake"

# before you need to run this:
# echo $GITHUB_REGISTRY_TOKEN | docker login ghcr.io -u <your_user> --password-stdin

# order seems to matter (same order as defined in compose file)
docker pull ghcr.io/rsksmart/union-client-builder:rust-1.86-v1 --platform linux/amd64
docker pull ghcr.io/rsksmart/union-client-block-indexer:latest --platform linux/amd64
docker pull ghcr.io/rsksmart/union-client-log-indexer:latest --platform linux/amd64
docker pull ghcr.io/rsksmart/union-client-transaction-dispatcher:latest --platform linux/amd64
docker pull ghcr.io/rsksmart/union-client-coordinator:latest --platform linux/amd64
docker pull ghcr.io/rsksmart/union-client-user-api:latest --platform linux/amd64
