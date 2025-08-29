#!/usr/bin/env bash

set -e

# TODO Improve receiving parameters and using "docker buildx bake"

# before you need to run this:
# echo $GITHUB_REGISTRY_TOKEN | docker login ghcr.io -u <your_user> --password-stdin

# order seems to matter (same order as defined in compose file)
docker tag union-client-builder:rust-1.86-v1 ghcr.io/rsksmart/union-client-builder:rust-1.86-v1
docker push ghcr.io/rsksmart/union-client-builder:rust-1.86-v1

docker tag union-client-block-indexer:latest ghcr.io/rsksmart/union-client-block-indexer:latest
docker push ghcr.io/rsksmart/union-client-block-indexer:latest

docker tag union-client-log-indexer:latest ghcr.io/rsksmart/union-client-log-indexer:latest
docker push ghcr.io/rsksmart/union-client-log-indexer:latest

docker tag union-client-transaction-dispatcher:latest ghcr.io/rsksmart/union-client-transaction-dispatcher:latest
docker push ghcr.io/rsksmart/union-client-transaction-dispatcher:latest

docker tag union-client-coordinator:latest ghcr.io/rsksmart/union-client-coordinator:latest
docker push ghcr.io/rsksmart/union-client-coordinator:latest

docker tag union-client-user-api:latest ghcr.io/rsksmart/union-client-user-api:latest
docker push ghcr.io/rsksmart/union-client-user-api:latest
