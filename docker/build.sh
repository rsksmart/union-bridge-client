#!/usr/bin/env bash

features=""
service=""

# build base image for caching (contains cargo workspace build)
docker build \
  --target builder \
  --ssh default="$SSH_AUTH_SOCK" \
  -t builder-union-client:latest \
  -f Dockerfile \
  .. || {
    echo "Failed to build builder-union-client image"
    exit 1
  }

for arg in "$@"; do
  if [[ "$arg" == "anvil" ]]; then
    features="anvil"
  elif [[ "$arg" =~ ^service=(.+)$ ]]; then
    service="${BASH_REMATCH[1]}"
  fi
done

export COMPOSE_PARALLEL_LIMIT=1

if [[ -n "$features" ]] && [[ -n "$service" ]]; then
  docker compose build "$service" --build-arg FEATURES="$features" --build-arg JUST_CRATE="$service"
elif [[ -n "$features" ]]; then
  docker compose build --build-arg FEATURES="$features"
elif [[ -n "$service" ]]; then
  docker compose build "$service" --build-arg JUST_CRATE="$service"
else
  docker compose build
fi

exit 0
