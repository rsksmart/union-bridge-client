#!/usr/bin/env bash

features=""
service=""

for arg in "$@"; do
  if [[ "$arg" == "anvil" ]]; then
    features="anvil"
  elif [[ "$arg" =~ ^service=(.+)$ ]]; then
    service="${BASH_REMATCH[1]}"
  fi
done

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
